/**
 * render.rs — POST /api/export/render
 *
 * Flujo:
 *   1. Llama al LLM para enriquecer el formato del texto (markdown estructurado)
 *      sin cambiar el contenido.  Si el LLM no está disponible usa el texto tal cual.
 *   2. Pasa el texto formateado a make_docx.js (Node.js) → DOCX binario.
 *   3. Para PDF: convierte el DOCX con LibreOffice soffice.
 *   4. Guarda en ~/.local-ai/workspace/ y revela el archivo en Finder.
 */

use axum::{
    Router,
    extract::State,
    response::IntoResponse,
    routing::post,
    http::StatusCode,
};
use serde::Deserialize;
use std::path::PathBuf;
use tracing::{error, info, warn};

use crate::AppState;

// ── Tipos ─────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct RenderRequest {
    pub text:   String,
    pub label:  String,
    pub format: String,
    pub title:  Option<String>,
    pub date:   Option<String>,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<AppState> {
    Router::new().route("/api/export/render", post(render_document))
}

// ── Handler ───────────────────────────────────────────────────────────────────

async fn render_document(
    State(state): State<AppState>,
    axum::Json(req): axum::Json<RenderRequest>,
) -> impl IntoResponse {
    let workspace = state.config.workspace_dir();

    if let Err(e) = tokio::fs::create_dir_all(&workspace).await {
        return (StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({"ok": false, "error": format!("workspace: {e}")}))).into_response();
    }

    // ── Paso 1: enriquecer formato vía LLM ────────────────────────────────────
    let formatted = format_with_llm(&req.text, &req.label, &state).await;

    let enriched_req = RenderRequest {
        text:   formatted,
        label:  req.label,
        format: req.format.clone(),
        title:  req.title,
        date:   req.date,
    };

    // ── Paso 2: generar documento ─────────────────────────────────────────────
    let result = match req.format.as_str() {
        "docx" => generate_docx(&enriched_req, &workspace).await,
        "pdf"  => generate_pdf(&enriched_req, &workspace).await,
        other  => Err(format!("Formato no soportado: {other}")),
    };

    match result {
        Err(e) => {
            error!("render/{}: {e}", req.format);
            (StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({"ok": false, "error": e}))).into_response()
        }
        Ok(saved_path) => {
            reveal_in_files(&saved_path);
            info!("render/{}: guardado → {}", req.format, saved_path.display());
            (StatusCode::OK, axum::Json(serde_json::json!({
                "ok":       true,
                "path":     saved_path.to_string_lossy(),
                "filename": saved_path.file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or(""),
            }))).into_response()
        }
    }
}

// ── Paso 1: Formateo inteligente con LLM ─────────────────────────────────────

/// Manda el texto al LLM con un prompt de formateo estructurado.
/// Devuelve el texto enriquecido con markdown; en caso de error devuelve el original.
async fn format_with_llm(text: &str, label: &str, state: &AppState) -> String {
    let snapshot = state.settings.read().unwrap().clone();
    let endpoint = snapshot.llm_endpoint.clone();
    let model    = if snapshot.llm_model.is_empty() { "llama3".to_string() } else { snapshot.llm_model.clone() };
    let api_key  = snapshot.api_key.clone();

    let system = format!(
r#"Eres un especialista en maquetación de documentos profesionales.
Tu ÚNICA tarea es añadir formato markdown estructurado al texto que recibes,
sin modificar ningún contenido, dato, cifra ni palabra.

Tipo de documento: {label}

== CONVENCIONES DE FORMATO QUE DEBES USAR ==

Estructura:
  ## Título de sección principal   (genera encabezado azul grande)
  ### Subtítulo / apartado         (genera encabezado más pequeño)

Énfasis inline:
  **texto**      → negrita (para etiquetas, nombres de entidad, palabras clave)
  *texto*        → cursiva (para términos técnicos, citas, extranjerismos)
  ***texto***    → negrita + cursiva (énfasis máximo)

Listas:
  - elemento     → lista con viñeta naranja
  1. elemento    → lista numerada

Tablas (cuando haya datos tabulares):
  | Columna 1 | Columna 2 | Columna 3 |
  |-----------|-----------|-----------|
  | valor     | valor     | valor     |

Separadores:
  ---            → línea divisoria entre secciones mayores
  ***            → separador decorativo centrado (p. ej. final de nota de prensa)

Metadatos / etiquetas de cabecera (PARA DIFUSIÓN INMEDIATA, datos de contacto…):
  Formatea como **ETIQUETA:** valor  (en su propia línea)

== REGLAS ABSOLUTAS ==
1. NO cambies, resumas ni parafrasees ningún contenido.
2. NO añadas texto nuevo ni explicaciones.
3. NO elimines información existente.
4. Devuelve ÚNICAMENTE el texto formateado, sin preámbulos ni comentarios.
5. Conserva todos los párrafos y saltos de línea originales."#
    );

    let user_msg = format!("{text}\n\n/no_think");

    let body = serde_json::json!({
        "model":       model,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user",   "content": user_msg},
        ],
        "stream":      false,
        "temperature": 0.2,
    });

    let client = reqwest::Client::new();
    let mut builder = client
        .post(format!("{endpoint}/v1/chat/completions"))
        .json(&body)
        .timeout(std::time::Duration::from_secs(120));

    if !api_key.is_empty() {
        builder = builder.header("Authorization", format!("Bearer {api_key}"));
    }

    match builder.send().await {
        Err(e) => {
            warn!("format_with_llm: LLM no disponible ({e}), usando texto original");
            text.to_string()
        }
        Ok(resp) => {
            if !resp.status().is_success() {
                warn!("format_with_llm: LLM status {}, usando texto original", resp.status());
                return text.to_string();
            }
            match resp.json::<serde_json::Value>().await {
                Err(e) => {
                    warn!("format_with_llm: parse error ({e}), usando texto original");
                    text.to_string()
                }
                Ok(json) => {
                    let content = json["choices"][0]["message"]["content"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();

                    if content.is_empty() {
                        warn!("format_with_llm: respuesta vacía, usando texto original");
                        text.to_string()
                    } else {
                        // Eliminar bloques <think>…</think> de Qwen 3
                        let cleaned = strip_think_tokens(&content);
                        info!("format_with_llm: OK ({} → {} chars)", text.len(), cleaned.len());
                        cleaned
                    }
                }
            }
        }
    }
}

/// Elimina bloques <think>…</think> y <channel|>* que genera Qwen 3.
/// Opera sobre &str para manejar UTF-8 correctamente.
fn strip_think_tokens(text: &str) -> String {
    // 1. Eliminar bloques <think>...</think>
    let mut result = String::with_capacity(text.len());
    let mut rest   = text;
    loop {
        match rest.find("<think>") {
            None => { result.push_str(rest); break; }
            Some(start) => {
                result.push_str(&rest[..start]);
                let after_open = &rest[start + 7..];
                match after_open.find("</think>") {
                    None    => break, // bloque sin cerrar → ignorar el resto
                    Some(e) => {
                        rest = after_open[e + 8..].trim_start_matches(|c| c == '\n' || c == '\r');
                    }
                }
            }
        }
    }

    // 2. Eliminar líneas que sean tokens <channel|>* de Qwen 3
    let filtered: Vec<&str> = result
        .lines()
        .filter(|l| !l.trim_start().starts_with("<channel|>"))
        .collect();

    filtered.join("\n").trim().to_string()
}

// ── Generar DOCX ──────────────────────────────────────────────────────────────

async fn generate_docx(req: &RenderRequest, dest_dir: &PathBuf) -> Result<PathBuf, String> {
    let bytes = run_make_docx(req).await?;
    let dest  = dest_dir.join(safe_filename(&req.label, "docx"));
    tokio::fs::write(&dest, &bytes).await.map_err(|e| format!("escribir docx: {e}"))?;
    Ok(dest)
}

// ── Generar PDF ───────────────────────────────────────────────────────────────

async fn generate_pdf(req: &RenderRequest, dest_dir: &PathBuf) -> Result<PathBuf, String> {
    let tmp = std::env::temp_dir().join(format!("oliv_{}", nonce()));
    tokio::fs::create_dir_all(&tmp).await.map_err(|e| format!("tmpdir: {e}"))?;

    let docx_bytes = run_make_docx(req).await?;
    let tmp_docx   = tmp.join("document.docx");
    tokio::fs::write(&tmp_docx, &docx_bytes).await.map_err(|e| format!("write tmp docx: {e}"))?;

    let soffice = find_soffice();
    info!("soffice: {soffice}");

    let out = tokio::process::Command::new(&soffice)
        .args(["--headless", "--convert-to", "pdf", "--outdir"])
        .arg(&tmp)
        .arg(&tmp_docx)
        .output()
        .await
        .map_err(|e| format!("soffice no encontrado — instala LibreOffice: {e}"))?;

    if !out.status.success() {
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        return Err(format!("soffice error: {}", String::from_utf8_lossy(&out.stderr)));
    }

    let pdf_data = tokio::fs::read(tmp.join("document.pdf")).await
        .map_err(|e| format!("leer pdf temporal: {e}"))?;
    let _ = tokio::fs::remove_dir_all(&tmp).await;

    let dest = dest_dir.join(safe_filename(&req.label, "pdf"));
    tokio::fs::write(&dest, &pdf_data).await.map_err(|e| format!("escribir pdf: {e}"))?;
    Ok(dest)
}

// ── make_docx.js vía Node.js ──────────────────────────────────────────────────

async fn run_make_docx(req: &RenderRequest) -> Result<Vec<u8>, String> {
    let script = find_make_docx_js()
        .ok_or_else(|| "make_docx.js no encontrado (busca en server/scripts/)".to_string())?;

    let tmp = std::env::temp_dir().join(format!("oliv_{}", nonce()));
    tokio::fs::create_dir_all(&tmp).await.map_err(|e| format!("tmpdir: {e}"))?;

    let json_path = tmp.join("input.json");
    let docx_path = tmp.join("output.docx");

    let input = serde_json::json!({
        "text":  req.text,
        "label": req.label,
        "title": req.title.as_deref().unwrap_or(req.label.as_str()),
        "date":  req.date.as_deref().unwrap_or(""),
    });
    tokio::fs::write(&json_path, input.to_string())
        .await.map_err(|e| format!("write json: {e}"))?;

    let out = tokio::process::Command::new("node")
        .arg(&script)
        .arg(&json_path)
        .arg(&docx_path)
        .output()
        .await
        .map_err(|e| format!("node no encontrado: {e}"))?;

    if !out.status.success() {
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        return Err(format!(
            "make_docx.js falló:\n{}\n{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout),
        ));
    }

    let bytes = tokio::fs::read(&docx_path).await
        .map_err(|e| format!("leer docx: {e}"))?;
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    info!("make_docx: {} bytes", bytes.len());
    Ok(bytes)
}

// ── Revelar en explorador ─────────────────────────────────────────────────────

fn reveal_in_files(path: &PathBuf) {
    #[cfg(target_os = "macos")]
    { let _ = std::process::Command::new("open").args(["-R", &path.to_string_lossy().to_string()]).spawn(); }
    #[cfg(target_os = "linux")]
    { let _ = std::process::Command::new("xdg-open").arg(path.parent().unwrap_or(path)).spawn(); }
}

// ── Localizar make_docx.js ────────────────────────────────────────────────────

fn find_make_docx_js() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("SCRIPTS_DIR") {
        let p = PathBuf::from(dir).join("make_docx.js");
        if p.exists() { return Some(p); }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(p) = exe.parent().and_then(|p| p.parent()).and_then(|p| p.parent()) {
            let c = p.join("scripts/make_docx.js");
            if c.exists() { return Some(c); }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        let c = cwd.join("server/scripts/make_docx.js");
        if c.exists() { return Some(c); }
        let c = cwd.join("scripts/make_docx.js");
        if c.exists() { return Some(c); }
    }
    None
}

// ── Localizar soffice ─────────────────────────────────────────────────────────

fn find_soffice() -> String {
    for c in &[
        "/Applications/LibreOffice.app/Contents/MacOS/soffice",
        "/opt/homebrew/opt/libreoffice/bin/soffice",
    ] {
        if std::path::Path::new(c).exists() { return c.to_string(); }
    }
    "soffice".to_string()
}

// ── Utilidades ────────────────────────────────────────────────────────────────

fn nonce() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn safe_filename(label: &str, ext: &str) -> String {
    let s: String = label.chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' })
        .collect();
    format!("{}.{}", s.trim(), ext)
}
