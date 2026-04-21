/**
 * render.rs — POST /api/export/render
 *
 * Pipeline 100% Rust: sin dependencias externas (Node, LibreOffice).
 * El texto llega en markdown desde /api/transform y se convierte directamente
 * a DOCX (docx-rs) o PDF (printpdf).
 */

use axum::{
    Router,
    extract::State,
    response::IntoResponse,
    routing::post,
    http::StatusCode,
};
use serde::Deserialize;
use std::path::Path;
use tracing::{error, info};

use crate::AppState;
use crate::routes::{render_docx, render_pdf};

#[derive(Deserialize)]
pub struct RenderRequest {
    pub text:     String,
    pub label:    String,
    pub format:   String,
    pub title:    Option<String>,
    pub date:     Option<String>,
    /// Si se provee, se guarda también una copia en
    /// `~/.local-ai/projects/{doc_hash}/outputs/` además del workspace.
    pub doc_hash: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/export/render",   post(render_document))
        .route("/api/export/download", post(download_document))
        .route("/api/export/save-as",  post(save_as_document))
}

#[derive(Deserialize)]
pub struct SaveAsRequest {
    pub text:     String,
    pub label:    String,
    pub format:   String, // "docx" | "pdf" | "txt" | "md"
    pub dest_dir: String,
    pub filename: String,
    pub title:    Option<String>,
    pub date:     Option<String>,
}

/// POST /api/export/save-as — genera el archivo y lo escribe en una ruta
/// elegida por el usuario a través del explorador custom del frontend.
/// Sustituye el diálogo "Guardar como" nativo del WebView cuando la app
/// corre en kiosk (sin acceso del usuario al filesystem real).
async fn save_as_document(
    State(state): State<AppState>,
    axum::Json(req): axum::Json<SaveAsRequest>,
) -> impl IntoResponse {
    let dest_dir = Path::new(&req.dest_dir);
    if !is_save_allowed(dest_dir, &state.config) {
        return (StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({"ok": false, "error": "Destino no permitido"}))).into_response();
    }
    if req.filename.is_empty()
        || req.filename.contains('/')
        || req.filename.contains('\\')
        || req.filename.contains("..")
    {
        return (StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({"ok": false, "error": "Nombre de archivo inválido"}))).into_response();
    }

    let title = req.title.as_deref().unwrap_or(req.label.as_str()).to_string();
    let date  = req.date.as_deref().unwrap_or("").to_string();
    let text  = req.text.clone();
    let label = req.label.clone();
    let format = req.format.clone();

    let bytes_res: Result<Vec<u8>, String> = match format.as_str() {
        "txt" | "md" => Ok(text.into_bytes()),
        "docx" | "pdf" => {
            let fmt = format.clone();
            let text = req.text.clone();
            tokio::task::spawn_blocking(move || -> Result<Vec<u8>, String> {
                match fmt.as_str() {
                    "docx" => render_docx::build_docx_bytes(&text, &label, &title, &date),
                    "pdf"  => render_pdf::build_pdf_bytes(&text, &label, &title, &date),
                    _      => unreachable!(),
                }
            }).await.map_err(|e| format!("panic en generador: {e}")).and_then(|r| r)
        }
        other => Err(format!("Formato no soportado: {other}")),
    };

    let bytes = match bytes_res {
        Ok(b)  => b,
        Err(e) => return render_error(&req.format, e),
    };

    if let Err(e) = tokio::fs::create_dir_all(dest_dir).await {
        return (StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({"ok": false, "error": format!("crear destino: {e}")}))).into_response();
    }

    let dest = dest_dir.join(&req.filename);
    if let Err(e) = tokio::fs::write(&dest, &bytes).await {
        return (StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({"ok": false, "error": format!("escribir: {e}")}))).into_response();
    }

    info!("save-as/{}: {} bytes → {}", req.format, bytes.len(), dest.display());

    (StatusCode::OK, axum::Json(serde_json::json!({
        "ok":   true,
        "path": dest.to_string_lossy(),
    }))).into_response()
}

fn is_save_allowed(path: &Path, config: &crate::config::Config) -> bool {
    if path.starts_with(&config.ai_base) { return true; }
    #[cfg(target_os = "macos")]
    if path.starts_with("/Volumes") { return true; }
    #[cfg(target_os = "linux")]
    if path.starts_with("/media") || path.starts_with("/mnt") { return true; }
    false
}

async fn render_document(
    State(state): State<AppState>,
    axum::Json(req): axum::Json<RenderRequest>,
) -> impl IntoResponse {
    let workspace = state.config.workspace_dir();

    if let Err(e) = tokio::fs::create_dir_all(&workspace).await {
        return (StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({"ok": false, "error": format!("workspace: {e}")}))).into_response();
    }

    let title = req.title.as_deref().unwrap_or(req.label.as_str()).to_string();
    let date  = req.date.as_deref().unwrap_or("").to_string();
    let text  = req.text.clone();
    let label = req.label.clone();
    let format = req.format.clone();

    // Generación CPU-bound: aislarla del runtime async.
    let result = tokio::task::spawn_blocking(move || -> Result<(Vec<u8>, &'static str), String> {
        match format.as_str() {
            "docx" => render_docx::build_docx_bytes(&text, &label, &title, &date).map(|b| (b, "docx")),
            "pdf"  => render_pdf::build_pdf_bytes(&text, &label, &title, &date).map(|b| (b, "pdf")),
            other  => Err(format!("Formato no soportado: {other}")),
        }
    }).await;

    let (bytes, ext) = match result {
        Ok(Ok(pair)) => pair,
        Ok(Err(e))   => return render_error(&req.format, e),
        Err(e)       => return render_error(&req.format, format!("panic en generador: {e}")),
    };

    let fname = safe_filename(&req.label, ext);
    let dest = workspace.join(&fname);
    if let Err(e) = tokio::fs::write(&dest, &bytes).await {
        return render_error(&req.format, format!("escribir {ext}: {e}"));
    }

    // Copia al proyecto si el frontend mandó doc_hash. Best-effort.
    if let Some(hash) = req.doc_hash.as_deref().filter(|h| !h.is_empty()) {
        let outputs = state.config.project_outputs_dir(hash);
        if let Err(e) = tokio::fs::create_dir_all(&outputs).await {
            tracing::warn!("project outputs create {}: {e}", outputs.display());
        } else {
            let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
            let stem = dest.file_stem().and_then(|s| s.to_str()).unwrap_or(&req.label);
            let project_file = outputs.join(format!("{stem}_{ts}.{ext}"));
            if let Err(e) = tokio::fs::write(&project_file, &bytes).await {
                tracing::warn!("project outputs write {}: {e}", project_file.display());
            } else {
                info!("render/{}: copia proyecto → {}", req.format, project_file.display());
            }
        }
    }

    reveal_in_files(&dest);
    info!("render/{}: guardado → {} ({} bytes)", req.format, dest.display(), bytes.len());

    (StatusCode::OK, axum::Json(serde_json::json!({
        "ok":       true,
        "path":     dest.to_string_lossy(),
        "filename": dest.file_name().and_then(|n| n.to_str()).unwrap_or(""),
    }))).into_response()
}

/// POST /api/export/download — genera DOCX/PDF y lo devuelve como
/// `Content-Disposition: attachment` para que el WebView dispare el diálogo
/// nativo de "Guardar como". No escribe en `workspace/`; sólo copia a
/// `projects/{doc_hash}/outputs/` si el frontend mandó `doc_hash`.
async fn download_document(
    State(state): State<AppState>,
    axum::Json(req): axum::Json<RenderRequest>,
) -> impl IntoResponse {
    let title = req.title.as_deref().unwrap_or(req.label.as_str()).to_string();
    let date  = req.date.as_deref().unwrap_or("").to_string();
    let text  = req.text.clone();
    let label = req.label.clone();
    let format = req.format.clone();

    let result = tokio::task::spawn_blocking(move || -> Result<(Vec<u8>, &'static str), String> {
        match format.as_str() {
            "docx" => render_docx::build_docx_bytes(&text, &label, &title, &date).map(|b| (b, "docx")),
            "pdf"  => render_pdf::build_pdf_bytes(&text, &label, &title, &date).map(|b| (b, "pdf")),
            other  => Err(format!("Formato no soportado: {other}")),
        }
    }).await;

    let (bytes, ext) = match result {
        Ok(Ok(pair)) => pair,
        Ok(Err(e))   => return render_error(&req.format, e),
        Err(e)       => return render_error(&req.format, format!("panic en generador: {e}")),
    };

    let fname = safe_filename(&req.label, ext);

    // Copia al proyecto si el frontend mandó doc_hash. Best-effort.
    if let Some(hash) = req.doc_hash.as_deref().filter(|h| !h.is_empty()) {
        let outputs = state.config.project_outputs_dir(hash);
        if let Err(e) = tokio::fs::create_dir_all(&outputs).await {
            tracing::warn!("project outputs create {}: {e}", outputs.display());
        } else {
            let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
            let stem = Path::new(&fname).file_stem().and_then(|s| s.to_str()).unwrap_or(&req.label);
            let project_file = outputs.join(format!("{stem}_{ts}.{ext}"));
            if let Err(e) = tokio::fs::write(&project_file, &bytes).await {
                tracing::warn!("project outputs write {}: {e}", project_file.display());
            } else {
                info!("download/{}: copia proyecto → {}", req.format, project_file.display());
            }
        }
    }

    let mime = match ext {
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "pdf"  => "application/pdf",
        _      => "application/octet-stream",
    };

    info!("download/{}: {} bytes → attachment {}", req.format, bytes.len(), fname);

    axum::response::Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", mime)
        .header(
            "Content-Disposition",
            format!("attachment; filename=\"{}\"", fname),
        )
        .header("Content-Length", bytes.len().to_string())
        .body(axum::body::Body::from(bytes))
        .unwrap()
}

fn render_error(format: &str, e: String) -> axum::response::Response {
    error!("render/{format}: {e}");
    (StatusCode::INTERNAL_SERVER_ERROR,
        axum::Json(serde_json::json!({"ok": false, "error": e}))).into_response()
}

fn reveal_in_files(path: &Path) {
    #[cfg(target_os = "macos")]
    { let _ = std::process::Command::new("open").args(["-R", &path.to_string_lossy().to_string()]).spawn(); }
    #[cfg(target_os = "linux")]
    { let _ = std::process::Command::new("xdg-open").arg(path.parent().unwrap_or(path)).spawn(); }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    { let _ = path; }
}

fn safe_filename(label: &str, ext: &str) -> String {
    let s: String = label.chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let trimmed = s.trim();
    let name = if trimmed.is_empty() { "documento" } else { trimmed };
    format!("{name}.{ext}")
}
