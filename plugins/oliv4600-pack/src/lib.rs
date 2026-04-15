use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{spawn_local, JsFuture};

// ─── View Enum ────────────────────────────────────────────────────────────────
// Each variant maps to a top-level workspace within the OLIV4600 interface.
// The active view is the single source of truth for which screen is rendered.
//
// Navigation mapping (top bar tabs → View):
//   "Editor"        → View::Editor      (3-col workspace: source | controls | output)
//   "Analysis"      → View::Analysis    (bento grid: readability, NER, sentiment, timeline)
//   "Chat"          → View::Chat        (50/50 split: document viewer + conversation panel)
//   "Verifiability" → View::Pipeline    (horizontal derivation chain / production pipeline)
//   "Audit"         → View::Audit       (tamper-evident operation log — Phase 8 placeholder)

#[derive(Clone, Copy, PartialEq)]
enum View {
    Dashboard,
    Editor,
    Analysis,
    Chat,
    Pipeline,
    Archive,
    Audit,
}

// ─── API response types ───────────────────────────────────────────────────────

use serde::{Deserialize, Serialize};

/// Row returned by GET /api/transformations
#[derive(Clone, Serialize, Deserialize)]
struct ApiTransform {
    #[allow(dead_code)]
    id:         i64,
    doc_name:   String,
    action:     String,
    #[allow(dead_code)]
    word_count: u32,
    created_at: String,
}

/// Proyecto del plugin — fila de oliv_projects
#[derive(Clone, Serialize, Deserialize)]
struct ApiProject {
    doc_hash:        String,
    doc_name:        String,
    #[allow(dead_code)]
    original_path:   String,
    word_count:      u32,
    transform_count: u32,
    has_analysis:    bool,
    created_at:      String,
    updated_at:      String,
}

/// Row returned by GET /api/documents
#[derive(Clone, Serialize, Deserialize)]
struct ApiDocument {
    doc_name:    String,
    #[allow(dead_code)]
    last_action: String,
    #[allow(dead_code)]
    count:       u32,
    #[allow(dead_code)]
    last_used:   String,
}

/// Wrapper for both API responses: { ok: bool, data: [...] }
#[derive(Serialize, Deserialize)]
struct ApiResponse<T> {
    #[allow(dead_code)]
    ok:   bool,
    data: Vec<T>,
}

// ─── API helper ───────────────────────────────────────────────────────────────

async fn fetch_json<T: for<'de> Deserialize<'de>>(url: &str) -> Option<T> {
    let window = web_sys::window()?;
    let resp: web_sys::Response = wasm_bindgen_futures::JsFuture::from(
        window.fetch_with_str(url)
    ).await.ok()?.unchecked_into();
    let text = wasm_bindgen_futures::JsFuture::from(resp.text().ok()?)
        .await.ok()?.as_string()?;
    serde_json::from_str(&text).ok()
}

// ─── Helpers de BD para plugins ──────────────────────────────────────────────
//
// El plugin gestiona sus propias tablas a través de los endpoints genéricos del
// core. Nunca accede directamente a SQLite — todo va por HTTP sobre localhost.
//
//   plugin_migrate(sql)           → POST /api/plugin/db/migrate
//   plugin_query(sql, params)     → POST /api/plugin/db/query  (SELECT/INSERT/...)

async fn plugin_migrate(sql: &str) -> bool {
    let body = serde_json::json!({ "sql": sql }).to_string();
    let headers = web_sys::Headers::new().unwrap();
    headers.set("Content-Type", "application/json").unwrap();
    let opts = web_sys::RequestInit::new();
    opts.set_method("POST");
    opts.set_body(&wasm_bindgen::JsValue::from_str(&body));
    opts.set_headers(&wasm_bindgen::JsValue::from(headers));
    let Ok(req) = web_sys::Request::new_with_str_and_init("/api/plugin/db/migrate", &opts)
        else { return false };
    let Some(window) = web_sys::window() else { return false };
    match JsFuture::from(window.fetch_with_request(&req)).await {
        Ok(rv) => { let r: web_sys::Response = rv.unchecked_into(); r.ok() },
        Err(_) => false,
    }
}

async fn plugin_query(
    sql:    &str,
    params: Vec<serde_json::Value>,
) -> Vec<serde_json::Value> {
    let body = serde_json::json!({ "sql": sql, "params": params }).to_string();
    let headers = web_sys::Headers::new().unwrap();
    headers.set("Content-Type", "application/json").unwrap();
    let opts = web_sys::RequestInit::new();
    opts.set_method("POST");
    opts.set_body(&wasm_bindgen::JsValue::from_str(&body));
    opts.set_headers(&wasm_bindgen::JsValue::from(headers));
    let Ok(req) = web_sys::Request::new_with_str_and_init("/api/plugin/db/query", &opts)
        else { return vec![] };
    let Some(window) = web_sys::window() else { return vec![] };
    let Ok(rv) = JsFuture::from(window.fetch_with_request(&req)).await else { return vec![] };
    let resp: web_sys::Response = rv.unchecked_into();
    if !resp.ok() { return vec![]; }
    let Ok(jv) = JsFuture::from(resp.json().unwrap()).await else { return vec![] };
    let Ok(s) = js_sys::JSON::stringify(&jv) else { return vec![] };
    let json_str = s.as_string().unwrap_or_default();
    serde_json::from_str::<serde_json::Value>(&json_str)
        .ok()
        .and_then(|v| v["rows"].as_array().cloned())
        .unwrap_or_default()
}

/// Map an action slug to a human-readable label.
fn action_badge(action: &str) -> (&'static str, &'static str) {
    match action {
        "executive_summary" | "detailed_summary" =>
            ("bg-[#003b65] text-[#66a6ea]",   "Summary"),
        "press_release"     =>
            ("bg-[#622700] text-[#fa813a]",   "Press Release"),
        "linkedin_post"     =>
            ("bg-[#003b65] text-[#66a6ea]",   "LinkedIn"),
        "academic_abstract" =>
            ("bg-surf-highest text-on-surf-var", "Abstract"),
        "blog_article"      =>
            ("bg-surf-highest text-on-surf-var", "Blog Article"),
        "briefing_note"     =>
            ("bg-surf-highest text-on-surf-var", "Briefing"),
        _                   =>
            ("bg-surf-highest text-on-surf-var", "Transform"),
    }
}

// ─── Document Context ─────────────────────────────────────────────────────────
// Compartido vía Leptos context (provide_context / use_context).
// Creado en App, leído/escrito por todas las vistas.

#[derive(Clone, Copy)]
struct DocumentCtx {
    /// Texto completo del documento cargado (ING-001..ING-003)
    text:           RwSignal<String>,
    /// Nombre del archivo cargado
    filename:       RwSignal<String>,
    /// Número de palabras
    word_count:     RwSignal<u32>,
    /// SHA-256 hex del texto — clave de proyecto en BD. Se rellena desde
    /// /api/extract (que lo calcula en servidor) o desde sha256_hex() en WASM.
    doc_hash:       RwSignal<String>,
    /// True mientras hay procesamiento en curso
    processing:     RwSignal<bool>,
    /// Texto generado (streaming token por token)
    output:         RwSignal<String>,
    /// Etiqueta de la última acción ejecutada
    output_label:   RwSignal<String>,
    /// Acción pre-seleccionada cuando se navega desde el Dashboard (slug)
    pending_action: RwSignal<Option<String>>,
}

impl DocumentCtx {
    fn new() -> Self {
        Self {
            text:           RwSignal::new(String::new()),
            filename:       RwSignal::new(String::new()),
            word_count:     RwSignal::new(0),
            doc_hash:       RwSignal::new(String::new()),
            processing:     RwSignal::new(false),
            output:         RwSignal::new(String::new()),
            output_label:   RwSignal::new(String::new()),
            pending_action: RwSignal::new(None),
        }
    }
}

// ─── Helper: upload + extracción de texto (ING-001, ING-002) ─────────────────
// POST /upload (multipart) → { path, name }
// POST /api/extract → { text, word_count, filename }
// En éxito: actualiza ctx y navega a View::Editor.

fn upload_and_load(file: web_sys::File, ctx: DocumentCtx, set_active_view: WriteSignal<View>) {
    spawn_local(async move {
        ctx.processing.set(true);

        // Step 1: upload
        let form_data = web_sys::FormData::new().unwrap();
        form_data.append_with_blob("file", &file).unwrap();
        let opts = web_sys::RequestInit::new();
        opts.set_method("POST");
        opts.set_body(&form_data.into());
        let request = web_sys::Request::new_with_str_and_init("/upload", &opts).unwrap();
        let window  = web_sys::window().unwrap();
        let resp: web_sys::Response = match JsFuture::from(window.fetch_with_request(&request)).await {
            Ok(r)  => r.unchecked_into(),
            Err(_) => { ctx.processing.set(false); return; }
        };
        let json_str = match JsFuture::from(resp.text().unwrap()).await {
            Ok(t) => t.as_string().unwrap_or_default(),
            Err(_) => { ctx.processing.set(false); return; }
        };
        let uploads: Vec<serde_json::Value> = serde_json::from_str(&json_str).unwrap_or_default();
        let path = match uploads.first().and_then(|u| u["path"].as_str()) {
            Some(p) => p.to_string(),
            None    => { ctx.processing.set(false); return; }
        };

        // Step 2: extract text
        let extract_body = serde_json::json!({"path": path}).to_string();
        let headers = web_sys::Headers::new().unwrap();
        headers.set("Content-Type", "application/json").unwrap();
        let opts2 = web_sys::RequestInit::new();
        opts2.set_method("POST");
        opts2.set_body(&wasm_bindgen::JsValue::from_str(&extract_body));
        opts2.set_headers(&wasm_bindgen::JsValue::from(headers));
        let req2 = web_sys::Request::new_with_str_and_init("/api/extract", &opts2).unwrap();
        let resp2: web_sys::Response = match JsFuture::from(window.fetch_with_request(&req2)).await {
            Ok(r)  => r.unchecked_into(),
            Err(_) => { ctx.processing.set(false); return; }
        };
        let json2 = match JsFuture::from(resp2.text().unwrap()).await {
            Ok(t) => t.as_string().unwrap_or_default(),
            Err(_) => { ctx.processing.set(false); return; }
        };
        if let Ok(ex) = serde_json::from_str::<serde_json::Value>(&json2) {
            let text     = ex["text"].as_str().unwrap_or("").to_string();
            let filename = ex["filename"].as_str().unwrap_or("").to_string();
            let doc_hash = ex["doc_hash"].as_str().unwrap_or("").to_string();
            let wc       = ex["word_count"].as_u64().unwrap_or(0) as u32;

            ctx.text.set(text);
            ctx.filename.set(filename.clone());
            ctx.doc_hash.set(doc_hash.clone());
            ctx.word_count.set(wc);

            // Registrar/actualizar proyecto en la tabla del plugin
            if !doc_hash.is_empty() {
                plugin_query(
                    "INSERT INTO oliv_projects \
                     (doc_hash, doc_name, original_path, word_count) \
                     VALUES (?1,?2,?3,?4) \
                     ON CONFLICT(doc_hash) DO UPDATE SET \
                       doc_name = excluded.doc_name, \
                       word_count = excluded.word_count, \
                       updated_at = strftime('%Y-%m-%dT%H:%M:%SZ','now')",
                    vec![
                        serde_json::json!(doc_hash),
                        serde_json::json!(filename),
                        serde_json::json!(path),
                        serde_json::json!(wc),
                    ],
                ).await;
            }

            set_active_view.set(View::Editor);
        }
        ctx.processing.set(false);
    });
}

// ─── Helper: llamada al motor de transformación (SRS §8.3) ───────────────────
// POST /api/transform con SSE streaming.
// Los tokens van acumulándose en ctx.output en tiempo real.

fn run_transform(
    ctx: DocumentCtx, action: String, length_words: u32, tone: u32,
    audience: String, language: String,
) {
    spawn_local(async move {
        ctx.processing.set(true);
        ctx.output.set(String::new());
        ctx.output_label.set(action_label(&action).to_string());
        let body = serde_json::json!({
            "text":         ctx.text.get_untracked(),
            "action":       action,
            "doc_name":     ctx.filename.get_untracked(),
            "length_words": length_words,
            "tone":         tone.to_string(),
            "audience":     audience,
            "language":     language,
        }).to_string();
        let headers = web_sys::Headers::new().unwrap();
        headers.set("Content-Type", "application/json").unwrap();
        let opts = web_sys::RequestInit::new();
        opts.set_method("POST");
        opts.set_body(&wasm_bindgen::JsValue::from_str(&body));
        opts.set_headers(&wasm_bindgen::JsValue::from(headers));
        let req = web_sys::Request::new_with_str_and_init("/api/transform", &opts).unwrap();
        let window = web_sys::window().unwrap();
        let resp: web_sys::Response = match JsFuture::from(window.fetch_with_request(&req)).await {
            Ok(r) => r.unchecked_into(),
            Err(_) => { ctx.processing.set(false); return; }
        };
        read_sse_stream(resp,
            move |t| ctx.output.update(|s| s.push_str(&t)),
            move || ctx.processing.set(false),
        ).await;
    });
}

// ─── Helper: chat sobre documento (CHA-001) ───────────────────────────────────
// POST /v1/chat/stream con contexto del documento como system prompt.

fn run_chat(ctx: DocumentCtx, messages: RwSignal<Vec<(String, String)>>, user_msg: String) {
    spawn_local(async move {
        ctx.processing.set(true);
        messages.update(|v| v.push(("user".to_string(), user_msg.clone())));
        let doc = ctx.text.get_untracked();
        let sys = if doc.is_empty() {
            "Eres OLIV4600, asistente de procesamiento documental. No hay documento cargado.".to_string()
        } else {
            format!(
                "Eres OLIV4600. El usuario ha cargado este documento:\n\n---\n{doc}\n---\n\n\
                 Responde preguntas sobre él. Cita el texto fuente cuando sea relevante."
            )
        };
        let msg_array: Vec<serde_json::Value> = std::iter::once(
            serde_json::json!({"role":"system","content":sys})
        ).chain(
            messages.get_untracked().iter().map(|(r, c)| serde_json::json!({"role":r,"content":c}))
        ).collect();
        let ai_idx = messages.get_untracked().len();
        messages.update(|v| v.push(("assistant".to_string(), String::new())));
        let body = serde_json::json!({"messages": msg_array, "stream": true}).to_string();
        let headers = web_sys::Headers::new().unwrap();
        headers.set("Content-Type", "application/json").unwrap();
        let opts = web_sys::RequestInit::new();
        opts.set_method("POST");
        opts.set_body(&wasm_bindgen::JsValue::from_str(&body));
        opts.set_headers(&wasm_bindgen::JsValue::from(headers));
        let req = web_sys::Request::new_with_str_and_init("/v1/chat/stream", &opts).unwrap();
        let window = web_sys::window().unwrap();
        let resp: web_sys::Response = match JsFuture::from(window.fetch_with_request(&req)).await {
            Ok(r) => r.unchecked_into(),
            Err(_) => { ctx.processing.set(false); return; }
        };
        read_sse_stream(resp,
            move |t| messages.update(|v| { if let Some(e) = v.get_mut(ai_idx) { e.1.push_str(&t); } }),
            move || ctx.processing.set(false),
        ).await;
    });
}

// ─── Helper: leer stream SSE ──────────────────────────────────────────────────
// Lee un ReadableStream de texto/event-stream, parsea eventos y llama a on_token
// por cada fragmento `event: token  data: {"text":"..."}` recibido.

async fn read_sse_stream<F: Fn(String), D: Fn()>(resp: web_sys::Response, on_token: F, on_done: D) {
    let reader: web_sys::ReadableStreamDefaultReader = match resp.body() {
        Some(b) => b.get_reader().unchecked_into(),
        None    => { on_done(); return; }
    };
    let mut buf = String::new();
    loop {
        let chunk = match JsFuture::from(reader.read()).await { Ok(c) => c, Err(_) => break };
        if js_sys::Reflect::get(&chunk, &wasm_bindgen::JsValue::from_str("done"))
            .ok().and_then(|v| v.as_bool()).unwrap_or(true) { break; }
        let value = match js_sys::Reflect::get(&chunk, &wasm_bindgen::JsValue::from_str("value")) {
            Ok(v) => v, Err(_) => break,
        };
        buf.push_str(&String::from_utf8_lossy(&js_sys::Uint8Array::new(&value).to_vec()));
        while let Some(idx) = buf.find("\n\n") {
            let msg = buf[..idx].to_string();
            buf = buf[idx + 2..].to_string();
            for line in msg.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    if let Ok(ev) = serde_json::from_str::<serde_json::Value>(data) {
                        if let Some(t) = ev["text"].as_str() { if !t.is_empty() { on_token(t.to_string()); } }
                    }
                }
            }
        }
    }
    on_done();
}

// ─── Helper: copiar al portapapeles (EXO-001) ─────────────────────────────────

fn copy_to_clipboard(text: String) {
    spawn_local(async move {
        if let Some(w) = web_sys::window() {
            // En esta versión de web-sys, clipboard() devuelve Clipboard directamente
            let cb = w.navigator().clipboard();
            let _ = JsFuture::from(cb.write_text(&text)).await;
        }
    });
}

// ─── Helper: etiqueta legible de una acción ───────────────────────────────────

fn action_label(action: &str) -> &'static str {
    match action {
        "executive_summary"     => "Resumen Ejecutivo",
        "technical_summary"     => "Resumen Técnico",
        "divulgative_summary"   => "Resumen Divulgativo",
        "bullet_summary"        => "Puntos Clave",
        "chronological_summary" => "Resumen Cronológico",
        "conclusions_summary"   => "Conclusiones y Recomendaciones",
        "briefing_2min"         => "Briefing 2 min",
        "press_release"         => "Nota de Prensa",
        "headlines"             => "Titulares",
        "linkedin_post"         => "Post LinkedIn",
        "twitter_thread"        => "Hilo Twitter/X",
        "blog_article"          => "Artículo de Blog",
        "instagram_post"        => "Post Instagram",
        "email_newsletter"      => "Email / Newsletter",
        "speech"                => "Discurso",
        "faqs"                  => "FAQs",
        "one_pager"             => "One-Pager / Ficha Resumen",
        "key_quotes"            => "Citas Textuales",
        "official_report"       => "Informe Oficial",
        "meeting_minutes"       => "Acta de Reunión",
        "administrative_resolution" => "Resolución Administrativa",
        "internal_memo"         => "Memorando Interno",
        "allegations_response"  => "Alegaciones / Negociación",
        "extract_commitments"   => "Compromisos Verificables",
        "rewrite_formal"        => "Reescritura Formal",
        "rewrite_shorter"       => "Reescritura Concisa",
        "rewrite_persuasive"    => "Reescritura Persuasiva",
        "rewrite_clearer"       => "Reescritura Clara",
        "detect_redundancies"   => "Detectar Redundancias",
        "translate_language"    => "Traducción",
        "sentiment_analysis"    => "Análisis de Sentimiento",
        "grammar_check"         => "Corrección Gramatical",
        "simplify"              => "Simplificar (Lenguaje Claro)",
        "detect_inconsistencies"=> "Detectar Inconsistencias",
        "reformulate_paragraph" => "Reformular Párrafo",
        "detect_ambiguities"    => "Detectar Ambigüedades",
        "improve_suggestions"   => "Sugerencias de Mejora",
        "readability_analysis"  => "Análisis de Legibilidad",
        "detect_evasive_language"=> "Lenguaje Evasivo",
        "semantic_versioning"   => "Versionado Semántico",
        "merge_documents"       => "Fusión de Documentos",
        "semantic_diff"         => "Diferencial Semántico",
        "document_intersection" => "Intersección Documental",
        "detect_contradictions" => "Detectar Contradicciones",
        "versions_compare"      => "Comparar Versiones",
        "inverse_questions"     => "Preguntas Inversas (Editor Jefe)",
        "press_release_check"   => "Verificar Nota de Prensa",
        "validation_questions"  => "Checklist de Validación",
        "ner_extraction"        => "Extracción de Entidades (NER)",
        "keywords_extraction"   => "Palabras Clave y Categorías",
        "event_timeline"        => "Línea Temporal",
        "impact_analysis"       => "Análisis de Impacto",
        "verifiability_check"   => "Verificabilidad y Soporte",
        "evidence_gaps"         => "Huecos de Evidencia",
        "traceability_map"      => "Mapa de Trazabilidad",
        "anonymize"             => "Anonimización / Expurgo",
        "preflight_check"       => "Preflight Documental",
        "public_version"        => "Versión Pública",
        "rgpd_check"            => "Verificación RGPD/LOPDGDD",
        "style_linting"         => "Linting Documental",
        "reader_simulation"     => "Simulador de Lector",
        "generate_from_form"    => "Generar desde Formulario",
        "generate_file_package" => "Paquete de Expediente",
        "crisis_press_questions"=> "Simulacro Comparecencia",
        "crisis_communication"  => "Kit de Crisis Reputacional",
        "argumentario"          => "Argumentario",
        "difficult_questions_simulator" => "Simulador Preguntas Difíciles",
        _                       => "Transformación",
    }
}

// ─── Helper: descarga un string como archivo (EXO-001, EXO-005) ──────────────

fn download_text(text: String, filename: &str, mime: &str) {
    use wasm_bindgen::JsValue;
    let Some(window) = web_sys::window() else { return };
    let Some(document) = window.document() else { return };
    // Crear Blob con el contenido
    let parts = js_sys::Array::new();
    parts.push(&JsValue::from_str(&text));
    let mut opts = web_sys::BlobPropertyBag::new();
    opts.type_(mime);
    let Ok(blob) = web_sys::Blob::new_with_str_sequence_and_options(&parts, &opts) else { return };
    let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else { return };
    // Crear <a> temporal y hacer click
    let Ok(el) = document.create_element("a") else { return };
    let a: web_sys::HtmlAnchorElement = el.unchecked_into();
    a.set_href(&url);
    a.set_download(filename);
    if let Some(body) = document.body() {
        let _ = body.append_child(&a);
        a.click();
        let _ = body.remove_child(&a);
    }
    let _ = web_sys::Url::revoke_object_url(&url);
}

// ─── Helper: guardar documento vía /api/export/render ────────────────────────
// El servidor escribe el archivo en ~/.local-ai/workspace/ y lo revela en
// Finder. WKWebView no soporta blob-downloads, por eso nunca descargamos
// binario al frontend: todo ocurre en el lado Rust.

async fn fetch_render(
    text:   String,
    label:  String,
    format: String,
    _fname: String,           // reservado; el servidor decide el nombre final
    toast:  RwSignal<Option<String>>,
) {
    use wasm_bindgen::JsValue;
    let body_json = serde_json::json!({
        "text":   text,
        "label":  label,
        "format": format,
    }).to_string();

    let headers = web_sys::Headers::new().unwrap();
    headers.set("Content-Type", "application/json").unwrap();

    let opts = web_sys::RequestInit::new();
    opts.set_method("POST");
    opts.set_body(&JsValue::from_str(&body_json));
    opts.set_headers(&JsValue::from(headers));

    let req  = web_sys::Request::new_with_str_and_init("/api/export/render", &opts).unwrap();
    let win  = web_sys::window().unwrap();

    let msg = match JsFuture::from(win.fetch_with_request(&req)).await {
        Err(_) => "Error de red al contactar el servidor.".to_string(),
        Ok(rv) => {
            let resp: web_sys::Response = rv.unchecked_into();
            if !resp.ok() {
                format!("Error del servidor ({})", resp.status())
            } else {
                // Leer JSON de confirmación
                match JsFuture::from(resp.json().unwrap()).await {
                    Err(_) => "Guardado (no se pudo leer respuesta).".to_string(),
                    Ok(jv) => {
                        // { ok, filename, path }
                        let fname = js_sys::Reflect::get(&jv, &JsValue::from_str("filename"))
                            .ok()
                            .and_then(|v| v.as_string())
                            .unwrap_or_default();
                        if fname.is_empty() {
                            "✓ Guardado en workspace".to_string()
                        } else {
                            format!("✓ Guardado: {fname}  (Finder abierto)")
                        }
                    }
                }
            }
        }
    };

    // Mostrar toast durante 4 segundos
    toast.set(Some(msg));
    let toast_clone = toast;
    gloo_timers::future::TimeoutFuture::new(4_000).await;
    toast_clone.set(None);
}

// ─── Helper: etiqueta de tono ─────────────────────────────────────────────────

fn tone_label(t: u32) -> &'static str {
    match t {
        1 => "Coloquial",
        2 => "Periodístico",
        3 => "Divulgativo",
        4 => "Técnico",
        5 => "Formal",
        _ => "Técnico",
    }
}

// ─── WASM entry point ─────────────────────────────────────────────────────────

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn main() {
    leptos::mount::mount_to_body(App);
}

// ─── Root component ───────────────────────────────────────────────────────────
// Owns all top-level reactive state and composes the layout shell.
//
// TODO: Add a `loaded_document: RwSignal<Option<Document>>` here so that every
// child view can read the currently loaded document without prop-drilling.
// Document should carry: raw_text, filename, language (ING-006), word_count.
//
// TODO: Add a `processing: RwSignal<bool>` for global loading overlay during
// LLM inference (PER-001 target: <15s for simple summaries).

#[component]
fn App() -> impl IntoView {
    // ── Migraciones del plugin ────────────────────────────────────────────────
    // Al arrancar, el plugin declara sus propias tablas vía el endpoint genérico
    // del core. El core no conoce estas tablas — solo ejecuta el DDL.
    spawn_local(async {
        plugin_migrate(r#"
            CREATE TABLE IF NOT EXISTS oliv_projects (
                doc_hash        TEXT    PRIMARY KEY,
                doc_name        TEXT    NOT NULL DEFAULT '',
                original_path   TEXT    NOT NULL DEFAULT '',
                word_count      INTEGER NOT NULL DEFAULT 0,
                transform_count INTEGER NOT NULL DEFAULT 0,
                has_analysis    INTEGER NOT NULL DEFAULT 0,
                created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
                updated_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
            );
            CREATE INDEX IF NOT EXISTS idx_oliv_projects_updated
                ON oliv_projects(updated_at DESC);
            CREATE TABLE IF NOT EXISTS oliv_analysis_cache (
                doc_hash        TEXT    PRIMARY KEY,
                doc_name        TEXT    NOT NULL DEFAULT '',
                word_count      INTEGER NOT NULL DEFAULT 0,
                readability_raw TEXT    NOT NULL DEFAULT '',
                sentiment_raw   TEXT    NOT NULL DEFAULT '',
                anomalies_raw   TEXT    NOT NULL DEFAULT '',
                ner_raw         TEXT    NOT NULL DEFAULT '',
                keywords_raw    TEXT    NOT NULL DEFAULT '',
                timeline_raw    TEXT    NOT NULL DEFAULT '',
                impact_raw      TEXT    NOT NULL DEFAULT '',
                created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
            );
        "#).await;
    });

    let (active_nav, set_active_nav) = signal("Projects");
    let (active_view, set_active_view) = signal(View::Dashboard);
    let (drag_over, set_drag_over)     = signal(false);

    // Contexto de documento compartido por todas las vistas
    let doc_ctx = DocumentCtx::new();
    provide_context(doc_ctx);

    view! {
        <div class="flex h-screen overflow-hidden bg-surface font-sans">
            <Sidebar active_nav set_active_nav set_active_view/>
            <div class="flex-1 flex flex-col overflow-hidden">
                <TopBar active_view set_active_view/>
                // Chat y Editor usan layout flex con scroll interno;
                // el resto hace scroll en este contenedor.
                <main class=move || match active_view.get() {
                    View::Chat | View::Editor => "flex-1 overflow-hidden",
                    _                         => "flex-1 overflow-y-auto",
                }>
                    // Overlay de procesamiento global
                    // Indicador de procesamiento — sólo en esquina, sin blur, sin bloquear clicks
                    {move || doc_ctx.processing.get().then(|| view! {
                        <div class="fixed top-20 right-6 z-[100] pointer-events-none">
                            <div class="bg-primary text-[#66a6ea] px-4 py-2 rounded-lg flex items-center gap-2 shadow-2xl">
                                <div class="w-2 h-2 rounded-full bg-[#66a6ea] animate-pulse"></div>
                                <span class="text-[10px] font-black uppercase tracking-widest">
                                    "Procesando..."
                                </span>
                            </div>
                        </div>
                    })}
                    {move || match active_view.get() {
                        View::Dashboard => view! { <DashboardView set_active_view drag_over set_drag_over/> }.into_any(),
                        View::Editor    => view! { <EditorView set_active_view/> }.into_any(),
                        View::Analysis  => view! { <AnalysisView/> }.into_any(),
                        View::Chat      => view! { <ChatView/> }.into_any(),
                        View::Pipeline  => view! { <PipelineView/> }.into_any(),
                        View::Archive   => view! { <ArchiveView set_active_view/> }.into_any(),
                        View::Audit     => view! { <AuditView/> }.into_any(),
                    }}
                </main>
            </div>
        </div>
    }
}

// ─── Sidebar ──────────────────────────────────────────────────────────────────
// Fixed left rail at #002542 (institutional navy). Intentionally minimal — this
// is a tool, not a consumer app.
//
// TODO (Module 12 — Templates & Context): Populate the Templates section
// dynamically from /api/templates, reading ~/.local-ai/templates/.
// Each template carries a document type schema used by the Guided Form
// Constructor (Module 16 — GUI-001..GUI-004).
//
// TODO (Projects): Load recent projects from /api/projects sorted by
// last-modified. Each project is a directory under ~/.local-ai/workspace/
// containing the source document + all derived outputs + audit log.

#[component]
fn Sidebar(
    active_nav:      ReadSignal<&'static str>,
    set_active_nav:  WriteSignal<&'static str>,
    set_active_view: WriteSignal<View>,
) -> impl IntoView {
    // ── Live recent documents from SQLite ─────────────────────────────────────
    // JsFuture is !Send (uses Rc internally), so we can't use Resource::new.
    // Instead: populate a signal via spawn_local (single-threaded WASM executor).
    // Proyectos recientes — el plugin consulta su propia tabla vía plugin_query
    let recent_docs: RwSignal<Option<Vec<ApiProject>>> = RwSignal::new(None);
    spawn_local(async move {
        let rows = plugin_query(
            "SELECT doc_hash, doc_name, original_path, word_count, \
             transform_count, has_analysis, created_at, updated_at \
             FROM oliv_projects ORDER BY updated_at DESC LIMIT 5",
            vec![],
        ).await;
        let projects = rows.into_iter().filter_map(|r| {
            Some(ApiProject {
                doc_hash:        r["doc_hash"].as_str()?.to_string(),
                doc_name:        r["doc_name"].as_str()?.to_string(),
                original_path:   r["original_path"].as_str()?.to_string(),
                word_count:      r["word_count"].as_u64()? as u32,
                transform_count: r["transform_count"].as_u64()? as u32,
                has_analysis:    r["has_analysis"].as_i64()? != 0,
                created_at:      r["created_at"].as_str()?.to_string(),
                updated_at:      r["updated_at"].as_str()?.to_string(),
            })
        }).collect::<Vec<_>>();
        recent_docs.set(Some(projects));
    });

    view! {
        <aside class="bg-[#002542] w-[280px] h-full flex flex-col py-8 px-4 shrink-0">
            // Clicking the brand mark returns to the Dashboard home screen.
            <div
                class="mb-10 px-2 cursor-pointer select-none"
                on:click=move |_| set_active_view.set(View::Dashboard)
            >
                <h1 class="text-2xl font-black tracking-tighter text-white">"OLIV4600"</h1>
                <p class="uppercase text-[11px] font-bold text-[#66a6ea] tracking-tight">
                    "Sovereign Intelligence"
                </p>
            </div>

            <nav class="flex-1 space-y-6 overflow-y-auto">
                <NavSection label="Main">
                    // TODO (Projects): navigating here should show the project browser —
                    // a grid of document cards with status badges (draft / processed / exported).
                    <NavItem icon="folder_open"  label="Projects"  active=active_nav set_active=set_active_nav/>
                    // TODO (Module 12 — Templates): show the template library where users can
                    // browse, preview, and apply organizational document templates (PLT-001..PLT-004).
                    <NavItem icon="edit_note"    label="Templates" active=active_nav set_active=set_active_nav/>
                    <div
                        class="flex items-center gap-3 px-4 py-2.5 rounded-lg cursor-pointer transition-colors text-slate-400 hover:text-white hover:bg-[#00335c]"
                        on:click=move |_| set_active_view.set(View::Archive)
                    >
                        <span class="material-symbols-outlined text-[20px]">"inventory_2"</span>
                        <span class="font-sans font-bold text-[11px] uppercase tracking-widest">"Library"</span>
                    </div>
                    // TODO (AI Engine): configuration panel for the local LLM instance —
                    // model selection, inference parameters (temperature, context window),
                    // Ollama endpoint health, and memory/disk usage stats.
                    <NavItem icon="memory"       label="AI Engine" active=active_nav set_active=set_active_nav/>
                </NavSection>

                <NavSection label="Recent Projects">
                    <div class="space-y-0.5 px-2">
                        {move || match recent_docs.get() {
                            None => view! {
                                <div class="text-[12px] text-slate-500 italic py-1 px-2">"Cargando…"</div>
                            }.into_any(),
                            Some(docs) if docs.is_empty() => view! {
                                <div class="text-[12px] text-slate-500 italic py-1 px-2">
                                    "Sin documentos recientes"
                                </div>
                            }.into_any(),
                            Some(docs) => docs.into_iter().map(|d| {
                                let name = d.doc_name.split('/').last()
                                    .unwrap_or(&d.doc_name)
                                    .to_string();
                                let label = if name.len() > 26 {
                                    format!("{}…", &name[..26])
                                } else { name };
                                let has_a = d.has_analysis;
                                view! {
                                    <div
                                        class="flex items-center gap-2 px-2 py-1.5 rounded cursor-pointer hover:bg-[#00335c] transition-colors group"
                                        on:click=move |_| set_active_view.set(View::Archive)
                                    >
                                        <span class="material-symbols-outlined text-[14px] text-slate-500 group-hover:text-slate-300 shrink-0">
                                            "description"
                                        </span>
                                        <span class="text-[12px] text-slate-400 group-hover:text-white truncate flex-1">
                                            {label}
                                        </span>
                                        // Badge análisis disponible
                                        {has_a.then(|| view! {
                                            <span class="w-1.5 h-1.5 rounded-full bg-[#66a6ea] shrink-0"></span>
                                        })}
                                    </div>
                                }
                            }).collect_view().into_any(),
                        }}
                    </div>
                </NavSection>

                <NavSection label="Templates">
                    <div class="space-y-0.5 px-2">
                        // TODO (Module 16 — GUI-003): clicking a template opens the Guided Form
                        // Constructor pre-filled with that document type's field schema, so the
                        // user can generate a full output package without an existing source file.
                        <TemplateItem label="Press Release"/>
                        <TemplateItem label="Internal Report"/>
                        <TemplateItem label="Meeting Minutes"/>
                    </div>
                </NavSection>
            </nav>

            <div class="mt-auto pt-6 space-y-4">
                // TODO: "New Document" should open a modal that lets the user choose:
                //   a) Upload file (PDF/DOCX/TXT/ODT/HTML — ING-001, ING-002)
                //      → POST /api/upload, extract text server-side (pdfextract / docx-rs)
                //   b) Inline rich-text editor (ING-003) — a Notion-lite editor embedded in the
                //      left panel of EditorView, supporting markdown shortcuts
                //   c) Guided Form Constructor (Module 16 — GUI-001) — structured fields
                //      whose values are injected into the LLM prompt as a virtual source doc
                // On completion, load document into global context → navigate to View::Editor.
                <button class="w-full bg-[#003b65] text-[#66a6ea] py-3 px-4 rounded-lg font-bold text-sm flex items-center justify-center gap-2 hover:bg-[#004d80] transition-colors active:scale-[0.98]">
                    <span class="material-symbols-outlined text-[18px]">"add"</span>
                    "New Document"
                </button>

                // ── Status Ribbon ─────────────────────────────────────────────
                // Hardware-label style indicators. Styled per Design System §5 "Status Ribbon":
                // 1.5px stroke feel, embossed on the UI, no decorative borders.
                //
                // TODO (SEC-001): Drive "System Offline" from a backend flag that verifies
                // no outbound HTTP connections are active. If the app is ever mis-configured
                // to call an external LLM endpoint, this badge should turn amber/red.
                //
                // TODO: "Qwen 3.5 Active" should be populated from GET /api/settings →
                // { llm_model, llm_endpoint }. Add a heartbeat ping (HEAD /api/chat) to
                // confirm the model is actually loaded; show a spinner if warming up.
                <div class="flex flex-col gap-2 border-t border-slate-700/50 pt-4">
                    <StatusRow icon="shield" color="text-[#66a6ea]" label="SYSTEM OFFLINE"/>
                    <StatusRowPulse icon="bolt" label="Qwen 3.5 Active"/>
                </div>
            </div>
        </aside>
    }
}

// ─── Sidebar sub-components ───────────────────────────────────────────────────

#[component]
fn NavSection(label: &'static str, children: Children) -> impl IntoView {
    view! {
        <div>
            <span class="px-2 mb-2 block uppercase text-[11px] font-bold text-slate-400 tracking-tight">
                {label}
            </span>
            {children()}
        </div>
    }
}

#[component]
fn NavItem(
    icon:       &'static str,
    label:      &'static str,
    active:     ReadSignal<&'static str>,
    set_active: WriteSignal<&'static str>,
) -> impl IntoView {
    view! {
        <button
            class=move || format!(
                "w-full flex items-center gap-3 px-3 py-2.5 rounded-lg text-left text-sm font-semibold transition-colors {}",
                if active.get() == label {
                    "bg-[#001b30] text-[#66a6ea] border-r-2 border-[#66a6ea]"
                } else {
                    "text-slate-400 hover:text-white hover:bg-[#00335c]"
                }
            )
            on:click=move |_| set_active.set(label)
        >
            <span class="material-symbols-outlined text-[20px]">{icon}</span>
            {label}
        </button>
    }
}

#[component]
fn RecentItem(label: String) -> impl IntoView {
    view! {
        <div class="text-[13px] text-slate-300 py-1 cursor-pointer hover:text-white truncate">
            {label}
        </div>
    }
}

#[component]
fn TemplateItem(label: &'static str) -> impl IntoView {
    view! {
        <div class="text-[13px] text-slate-300 py-1 cursor-pointer hover:text-white flex items-center gap-2">
            <span class="w-1.5 h-1.5 rounded-full bg-slate-500 shrink-0"></span>
            {label}
        </div>
    }
}

#[component]
fn StatusRow(icon: &'static str, color: &'static str, label: &'static str) -> impl IntoView {
    view! {
        <div class="flex items-center gap-3 px-2 py-1.5">
            <span class=format!("material-symbols-outlined text-[18px] {color}")>{icon}</span>
            <span class=format!("uppercase text-[10px] font-bold {color} tracking-widest")>{label}</span>
        </div>
    }
}

#[component]
fn StatusRowPulse(icon: &'static str, label: &'static str) -> impl IntoView {
    view! {
        <div class="flex items-center gap-3 px-2 py-1.5">
            <div class="relative">
                <span class="material-symbols-outlined text-[18px] text-green-400">{icon}</span>
                <span class="absolute -top-1 -right-1 w-2 h-2 bg-green-400 rounded-full animate-pulse"></span>
            </div>
            <span class="uppercase text-[10px] font-bold text-white tracking-widest">{label}</span>
        </div>
    }
}

// ─── Top Bar ──────────────────────────────────────────────────────────────────
// Shared header present across all views. Tab navigation changes the active view.
// The active tab is visually marked with the action color (#C45911) underline.
//
// TODO: The search bar should query the document archive using a full-text index
// (SQLite FTS5) exposed at GET /api/search?q=... — results should appear in a
// floating dropdown with document name, excerpt, and last-modified date.
//
// TODO: The "Execute Process" button should be context-sensitive:
//   - On Dashboard: disabled (greyed) until a document is loaded
//   - On Editor: triggers the selected transformation with current parameters
//   - On Analysis: re-runs the full forensic analysis suite on the loaded document
//   - On Pipeline: regenerates all derived outputs from the source

#[component]
fn TopBar(
    active_view:     ReadSignal<View>,
    set_active_view: WriteSignal<View>,
) -> impl IntoView {
    view! {
        <header class="bg-white/85 backdrop-blur-xl flex justify-between items-center h-16 px-8 shrink-0 border-b border-slate-200/50 shadow-sm sticky top-0 z-40">
            <div class="flex items-center gap-8">
                <div class="relative flex items-center">
                    <span class="material-symbols-outlined absolute left-3 text-slate-400 text-[20px]">"search"</span>
                    // TODO: wire to GET /api/search?q={value} with 300ms debounce
                    <input
                        class="pl-10 pr-4 py-2 bg-surf-low border-none rounded-lg text-sm w-64 focus:outline-none focus:ring-1 focus:ring-primary"
                        placeholder="Search archive..."
                        type="text"
                    />
                </div>
                <nav class="flex gap-6">
                    <TopTab label="Editor"        view=View::Editor        active=active_view set_active=set_active_view/>
                    <TopTab label="Analysis"      view=View::Analysis      active=active_view set_active=set_active_view/>
                    <TopTab label="Chat"          view=View::Chat          active=active_view set_active=set_active_view/>
                    <TopTab label="Verifiability" view=View::Pipeline      active=active_view set_active=set_active_view/>
                    <TopTab label="Audit"         view=View::Audit         active=active_view set_active=set_active_view/>
                </nav>
            </div>
            <div class="flex items-center gap-4">
                <div class="flex gap-2 mr-4 border-r border-slate-200 pr-4">
                    // TODO (SEC-005): history_edu icon → opens the Audit Log (View::Audit),
                    // showing the last N operations with timestamps and SHA-256 output hashes.
                    <IconBtn icon="shield"/>
                    <IconBtn icon="history_edu"/>
                    // TODO: settings icon → opens the Settings panel (config_panel equivalent)
                    // for LLM endpoint, model, org identity (PLT-002), and export preferences.
                    <IconBtn icon="settings"/>
                </div>
                // TODO: context-sensitive action — see comment above TopBar component
                <button class="px-4 py-2 bg-primary text-white rounded-lg font-bold text-sm hover:bg-[#003b65] transition-colors active:opacity-80">
                    "Execute Process"
                </button>
            </div>
        </header>
    }
}

#[component]
fn TopTab(
    label:      &'static str,
    view:       View,
    active:     ReadSignal<View>,
    set_active: WriteSignal<View>,
) -> impl IntoView {
    view! {
        <button
            class=move || format!(
                "font-serif italic text-lg transition-all {}",
                if active.get() == view {
                    "text-action border-b-2 border-action pb-1"
                } else {
                    "text-slate-500 hover:text-primary"
                }
            )
            on:click=move |_| set_active.set(view)
        >
            {label}
        </button>
    }
}

#[component]
fn IconBtn(icon: &'static str) -> impl IntoView {
    view! {
        <button class="p-2 text-slate-500 hover:bg-surf-cont rounded-lg transition-colors">
            <span class="material-symbols-outlined">{icon}</span>
        </button>
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// VIEW: DASHBOARD (Home)
// ═══════════════════════════════════════════════════════════════════════════════
// Landing screen shown before any document is loaded.
// Provides the primary document ingestion entry point (drag-drop + file type buttons)
// and quick-action shortcuts to the most common transformation types.
//
// Design: asymmetric bento grid with a hero upload zone, 5-col action grid,
// and a recent-transformations table. Floating "Sovereign Protocol Active" pill
// at the bottom of the upload zone acts as the Status Ribbon per DESIGN.md §5.

#[component]
fn DashboardView(
    set_active_view: WriteSignal<View>,
    drag_over:       ReadSignal<bool>,
    set_drag_over:   WriteSignal<bool>,
) -> impl IntoView {
    let ctx            = use_context::<DocumentCtx>().expect("DocumentCtx");
    let file_input_ref = NodeRef::<leptos::html::Input>::new();

    // ── Live transformations from SQLite ──────────────────────────────────────
    // JsFuture is !Send — use spawn_local + RwSignal instead of Resource.
    let transforms: RwSignal<Option<Vec<ApiTransform>>> = RwSignal::new(None);
    spawn_local(async move {
        let data = fetch_json::<ApiResponse<ApiTransform>>("/api/transformations?limit=20")
            .await
            .map(|r| r.data)
            .unwrap_or_default();
        transforms.set(Some(data));
    });

    view! {
        <section class="p-8 max-w-7xl mx-auto space-y-8">

            // Input de archivo oculto — lo activan los botones y el drag-drop
            // (ING-001, ING-002)
            <input
                type="file"
                accept=".pdf,.docx,.txt,.odt,.html,.htm,.md,.csv"
                class="hidden"
                node_ref=file_input_ref
                on:change=move |_| {
                    if let Some(input) = file_input_ref.get() {
                        if let Some(files) = input.files() {
                            if let Some(file) = files.get(0) {
                                upload_and_load(file, ctx, set_active_view);
                            }
                        }
                    }
                }
            />

            // ── Hero: Drag & Drop Zone (ING-001, ING-002) ─────────────────────
            <div class="relative">
                <div
                    class=move || format!(
                        "bg-white border-2 border-dashed rounded-xl p-16 text-center flex flex-col items-center justify-center transition-all group {}",
                        if drag_over.get() { "border-primary bg-[#002542]/5" }
                        else               { "border-outline-var hover:border-[#66a6ea]" }
                    )
                    on:dragover=move |e| { e.prevent_default(); set_drag_over.set(true); }
                    on:dragleave=move |_| set_drag_over.set(false)
                    on:drop=move |e: leptos::ev::DragEvent| {
                        e.prevent_default();
                        set_drag_over.set(false);
                        if let Some(dt) = e.data_transfer() {
                            if let Some(files) = dt.files() {
                                if let Some(file) = files.get(0) {
                                    upload_and_load(file, ctx, set_active_view);
                                }
                            }
                        }
                    }
                >
                    <div class="w-16 h-16 bg-surf-low rounded-full flex items-center justify-center mb-6 text-primary group-hover:scale-110 transition-transform">
                        <span class="material-symbols-outlined text-[32px]">"upload_file"</span>
                    </div>
                    <h2 class="font-serif italic text-3xl text-primary mb-2">
                        "Arrastra un documento o empieza a escribir"
                    </h2>
                    <p class="text-on-surf-var text-sm mb-8">
                        "Procesamiento 100% local. Tus datos nunca salen de este equipo."
                    </p>
                    <div class="flex flex-wrap justify-center gap-3">
                        // ING-001: PDF
                        <FileBtn icon="picture_as_pdf" label="PDF"
                            on_click=move || { if let Some(i) = file_input_ref.get() { i.click(); } }
                        />
                        // ING-002: DOCX / TXT / ODT
                        <FileBtn icon="description" label="DOCX"
                            on_click=move || { if let Some(i) = file_input_ref.get() { i.click(); } }
                        />
                        <FileBtn icon="article" label="TXT"
                            on_click=move || { if let Some(i) = file_input_ref.get() { i.click(); } }
                        />
                        // ING-003: editor en blanco
                        <button
                            class="flex items-center gap-2 px-5 py-2.5 bg-primary text-white rounded-lg font-bold text-xs uppercase tracking-wider shadow-lg shadow-primary/20 hover:-translate-y-0.5 transition-all"
                            on:click=move |_| {
                                ctx.text.set(String::new());
                                ctx.filename.set("Nuevo documento".to_string());
                                ctx.word_count.set(0);
                                set_active_view.set(View::Editor);
                            }
                        >
                            <span class="material-symbols-outlined text-[18px]">"add_circle"</span>
                            "Nuevo Documento"
                        </button>
                    </div>
                </div>
                // Status Ribbon — always visible below the drop zone
                <div class="absolute -bottom-3 left-1/2 -translate-x-1/2 px-4 py-1.5 bg-primary text-[#66a6ea] border border-[#66a6ea]/30 rounded-full flex items-center gap-2 shadow-xl whitespace-nowrap">
                    <div class="w-1.5 h-1.5 bg-[#66a6ea] rounded-full animate-pulse"></div>
                    <span class="text-[10px] font-black uppercase tracking-widest">
                        "Sovereign Protocol Active"
                    </span>
                </div>
            </div>

            // ── Quick Actions Grid (asymmetric 5-col) ──────────────────────────
            // These cards are shortcuts into specific transformation modules.
            // Clicking any card should: check if a document is loaded → if not,
            // prompt the user to upload first → if yes, navigate to EditorView
            // with the corresponding action pre-selected in the control panel.
            <div class="grid grid-cols-1 md:grid-cols-5 gap-6">
                // Feature card (2-col wide) — primary CTA for advanced processing
                // TODO: "Initialize Engine" should run a system check:
                //   1. Ping Ollama endpoint → verify model is loaded
                //   2. Check available VRAM/RAM for the selected model
                //   3. Display a readiness checklist in a modal
                <div class="md:col-span-2 bg-primary text-white p-8 rounded-xl flex flex-col justify-between min-h-[240px] relative overflow-hidden group">
                    <div class="relative z-10">
                        <span class="material-symbols-outlined text-[40px] text-[#66a6ea] mb-4 block">"auto_awesome"</span>
                        <h3 class="font-serif italic text-2xl mb-2">"Advanced Transformation"</h3>
                        <p class="text-slate-400 text-sm leading-relaxed max-w-xs">
                            "Convert raw data into professional intelligence reports using the Qwen 3.5 engine."
                        </p>
                    </div>
                    <button class="relative z-10 mt-6 self-start px-4 py-2 bg-[#66a6ea] text-primary rounded-lg font-bold text-xs uppercase hover:scale-105 active:scale-95 transition-transform">
                        "Initialize Engine"
                    </button>
                    <div class="absolute right-[-20px] bottom-[-20px] opacity-10 group-hover:scale-110 transition-transform duration-700 pointer-events-none">
                        <span class="material-symbols-outlined text-[200px]">"memory"</span>
                    </div>
                </div>
                <ActionCard icon="summarize" icon_color="text-action"
                    title="Summarize" desc="Executive distillation of core concepts."
                    on_click=move || {
                        ctx.pending_action.set(Some("executive_summary".to_string()));
                        set_active_view.set(View::Editor);
                    }
                />
                <ActionCard icon="campaign" icon_color="text-action"
                    title="Press Release" desc="Draft professional public communications."
                    on_click=move || {
                        ctx.pending_action.set(Some("press_release".to_string()));
                        set_active_view.set(View::Editor);
                    }
                />
                <ActionCard icon="forum" icon_color="text-[#66a6ea]"
                    title="Chat with Doc" desc="Direct query dialogue with your source text."
                    on_click=move || set_active_view.set(View::Chat)
                />
            </div>

            // ── Recent Transformations Table ───────────────────────────────────
            // Shows the N most recent processing operations across all projects.
            // TODO: Fetch from GET /api/projects?limit=10&sort=updated_at (TRA-001).
            // Each row is clickable to re-open that project in EditorView with the
            // result pre-loaded in the output panel.
            // TODO (EXO-001..EXO-006): the download icon should open an export modal
            // with format options: DOCX, PDF, HTML, Markdown, clipboard (copy).
            <div class="space-y-4">
                <div class="flex justify-between items-end">
                    <div>
                        <h3 class="font-sans font-black text-xl text-primary tracking-tighter">
                            "Recent Transformations"
                        </h3>
                        <p class="font-serif italic text-on-surf-var text-sm">
                            "History of processed intelligence"
                        </p>
                    </div>
                    <button
                        on:click=move |_| set_active_view.set(View::Archive)
                        class="text-xs font-bold uppercase tracking-widest text-primary flex items-center gap-1 hover:underline"
                    >
                        "View Full Archive"
                        <span class="material-symbols-outlined text-[16px]">"arrow_forward"</span>
                    </button>
                </div>
                <div class="bg-white rounded-xl shadow-sm overflow-hidden border border-slate-200/50">
                    <table class="w-full text-left border-collapse">
                        <thead>
                            <tr class="bg-surf-low border-b border-slate-200">
                                <Th>"Document Name"</Th>
                                <Th>"Transformation Type"</Th>
                                <Th>"Timestamp"</Th>
                                <th class="px-6 py-4 font-sans font-bold text-[10px] uppercase tracking-widest text-slate-500 text-right">
                                    "Actions"
                                </th>
                            </tr>
                        </thead>
                        <tbody class="divide-y divide-slate-100">
                            {move || match transforms.get() {
                                None => view! {
                                    <tr>
                                        <td colspan="4" class="px-6 py-8 text-center text-xs text-slate-400 italic">
                                            "Cargando historial…"
                                        </td>
                                    </tr>
                                }.into_any(),
                                Some(rows) if rows.is_empty() => view! {
                                    <tr>
                                        <td colspan="4" class="px-6 py-8 text-center text-xs text-slate-400 italic">
                                            "No hay transformaciones todavía. Procesa un documento para empezar."
                                        </td>
                                    </tr>
                                }.into_any(),
                                Some(rows) => rows.into_iter().map(|t| {
                                    let (badge, label) = action_badge(&t.action);
                                    view! {
                                        <TxRow
                                            doc_name=t.doc_name
                                            kind=label.to_string()
                                            badge=badge.to_string()
                                            ts=t.created_at
                                        />
                                    }
                                }).collect_view().into_any(),
                            }}
                        </tbody>
                    </table>
                </div>
            </div>

            // ── Footer ─────────────────────────────────────────────────────────
            // Model signature and emergency controls.
            // TODO (SEC-002 — AES-256): "Emergency Purge" should call DELETE /api/purge
            // which wipes ~/.local-ai/workspace/ and the SQLite audit log, then
            // shows a confirmation banner. Require a two-step confirmation dialog.
            <footer class="pb-8 border-t border-slate-200 pt-8">
                <div class="flex flex-col md:flex-row justify-between items-center gap-6">
                    <div class="flex items-center gap-8">
                        <div class="flex flex-col">
                            <span class="text-[10px] font-bold uppercase tracking-widest text-slate-400">
                                "Computational Engine"
                            </span>
                            // TODO: populate from GET /api/settings → llm_model field
                            <span class="font-sans font-bold text-primary">"Qwen 3.5 — 32B Instruct"</span>
                        </div>
                        <div class="h-8 w-[1px] bg-slate-200 hidden md:block"></div>
                        <div class="flex flex-col">
                            <span class="text-[10px] font-bold uppercase tracking-widest text-slate-400">
                                "Privacy Status"
                            </span>
                            <span class="font-sans font-bold text-primary flex items-center gap-2">
                                <span class="w-2 h-2 bg-green-500 rounded-full"></span>
                                "100% Air-Gapped"
                            </span>
                        </div>
                    </div>
                    <div class="flex gap-4">
                        // TODO (TRA-003): export the full audit log as JSON/CSV
                        <button class="px-4 py-2 text-primary border border-primary rounded-lg text-xs font-bold uppercase tracking-widest hover:bg-primary hover:text-white transition-all">
                            "Export Audit Log"
                        </button>
                        <button class="px-4 py-2 bg-primary text-white rounded-lg text-xs font-bold uppercase tracking-widest active:opacity-80 transition-all">
                            "Emergency Purge"
                        </button>
                    </div>
                </div>
            </footer>
        </section>

        // Floating Action Button — visible when scrolled down, shortcut to "New Document"
        // TODO: conditionally show only when the user has scrolled past the drop zone.
        <div class="fixed bottom-8 right-8 z-50">
            <button
                class="w-14 h-14 bg-primary text-white rounded-full flex items-center justify-center shadow-2xl hover:scale-110 active:scale-95 transition-all"
                on:click=move |_| set_active_view.set(View::Editor)
            >
                <span class="material-symbols-outlined text-[28px]">"add"</span>
            </button>
        </div>
    }
}

// Dashboard sub-components

#[component]
fn FileBtn(icon: &'static str, label: &'static str, on_click: impl Fn() + 'static) -> impl IntoView {
    view! {
        <button
            class="flex items-center gap-2 px-5 py-2.5 bg-surf-high rounded-lg font-bold text-xs uppercase tracking-wider text-primary hover:bg-surf-highest transition-colors"
            on:click=move |_| on_click()
        >
            <span class="material-symbols-outlined text-[18px]">{icon}</span>
            {label}
        </button>
    }
}

#[component]
fn ActionCard(
    icon:       &'static str,
    icon_color: &'static str,
    title:      &'static str,
    desc:       &'static str,
    on_click:   impl Fn() + 'static,
) -> impl IntoView {
    view! {
        <div
            class="bg-white p-6 rounded-xl shadow-sm border border-transparent hover:border-outline-var hover:shadow-md transition-all cursor-pointer group active:scale-[0.98]"
            on:click=move |_| on_click()
        >
            <span class=format!("material-symbols-outlined {icon_color} mb-4 block")>{icon}</span>
            <h4 class="font-sans font-bold text-sm uppercase tracking-tight mb-1 text-primary">{title}</h4>
            <p class="text-on-surf-var text-xs font-serif italic">{desc}</p>
        </div>
    }
}

#[component]
fn Th(children: Children) -> impl IntoView {
    view! {
        <th class="px-6 py-4 font-sans font-bold text-[10px] uppercase tracking-widest text-slate-500">
            {children()}
        </th>
    }
}

#[component]
fn TxRow(doc_name: String, kind: String, badge: String, ts: String) -> impl IntoView {
    // Show only the filename part (strip path prefixes)
    let short_name = doc_name.split('/').last()
        .unwrap_or(&doc_name)
        .to_string();
    // Format the ISO timestamp to something readable, e.g. "2026-04-08T14:32:00Z" → "Apr 08, 2026 — 14:32"
    let display_ts = ts.get(..16).unwrap_or(&ts).replace('T', " — ");

    view! {
        <tr class="hover:bg-surf-low transition-colors group">
            <td class="px-6 py-4">
                <div class="flex items-center gap-3">
                    <span class="material-symbols-outlined text-slate-400">"description"</span>
                    <span class="font-sans font-bold text-sm text-primary">{short_name}</span>
                </div>
            </td>
            <td class="px-6 py-4">
                <span class=format!("px-2.5 py-1 {} rounded text-[10px] font-black uppercase tracking-tighter", badge)>
                    {kind}
                </span>
            </td>
            <td class="px-6 py-4 text-xs text-on-surf-var">{display_ts}</td>
            <td class="px-6 py-4 text-right">
                <div class="flex justify-end gap-2 opacity-0 group-hover:opacity-100 transition-opacity">
                    <RowBtn icon="visibility"/>
                    <RowBtn icon="download"/>
                </div>
            </td>
        </tr>
    }
}

#[component]
fn RowBtn(icon: &'static str) -> impl IntoView {
    view! {
        <button class="p-1.5 hover:bg-surf-cont rounded-lg text-primary transition-colors">
            <span class="material-symbols-outlined text-[18px]">{icon}</span>
        </button>
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// VIEW: EDITOR (Workspace Core)
// ═══════════════════════════════════════════════════════════════════════════════
// The primary transformation workspace. Three-column layout:
//   Left  (45%, blue accent border): Source document — editable or read-only view
//   Middle (240px, neutral):         Control panel — action selector + parameters
//   Right (flex-1, orange accent):   Result area — AI-generated output
//
// This view implements the core loop of OLIV4600:
//   Source text → parametrized LLM prompt → formatted professional output
//
// Corresponds to SRS modules: 2 (Summaries), 3 (Derived Content), 5 (Tone),
// 6 (Assisted Editing), 12 (Templates), 13 (Verifiability), 14 (Safe Publication).

#[component]
fn EditorView(set_active_view: WriteSignal<View>) -> impl IntoView {
    let ctx = use_context::<DocumentCtx>().expect("DocumentCtx");

    // ── Parámetros del panel de control ──────────────────────────────────────
    // Pre-select action if the Dashboard navigated here with a pending_action.
    let initial_action = ctx.pending_action
        .get_untracked()
        .unwrap_or_else(|| "executive_summary".to_string());
    // Consume the pending_action so it doesn't re-apply on future mounts.
    ctx.pending_action.set(None);

    let (selected_action, set_selected_action) = signal(initial_action);
    let (length_words,    set_length)          = signal(250u32);
    let (tone,            set_tone)            = signal(4u32);
    let (audience,        set_audience)        = signal("technical".to_string());
    let (language,        set_language)        = signal("es".to_string());
    let _ = set_language; // expuesto en la UI futura (TON-002)

    // Señal derivada: ¿hay resultado para mostrar?
    let has_output = move || !ctx.output.get().is_empty();
    // Modal de exportación
    let show_export = RwSignal::new(false);
    // Toast de confirmación de guardado
    let save_toast: RwSignal<Option<String>> = RwSignal::new(None);

    view! {
        <div class="h-full flex overflow-hidden">

            // ── Column 1: Source Document ──────────────────────────────────────
            // The left panel displays the loaded source document.
            // The 4px blue left border visually signals "raw input / authority source".
            //
            // TODO (ING-003): When no file is loaded, render a contenteditable rich-text
            // editor here instead of static text. Support markdown shortcuts and basic
            // formatting (bold, italic, headers, lists) via a lightweight JS interop layer.
            //
            // TODO (Module 6 — EDI-001..EDI-007): The text should be spell-checked
            // in real time (underline squiggly on errors). Right-click context menu
            // should offer: "Simplify", "Reformulate", "Detect ambiguity" for the
            // selected sentence/paragraph.
            // ── Columna 1: Documento Fuente ───────────────────────────────────
            <section class="w-[45%] flex flex-col border-l-[4px] border-[#2E75B6] bg-white">
                // Cabecera del panel fuente
                <div class="h-12 border-b border-slate-100 flex items-center justify-between px-6 bg-surf-low/30">
                    <div class="flex items-center gap-4">
                        <span class="text-[10px] font-bold uppercase tracking-tighter text-outline">
                            {move || {
                                let f = ctx.filename.get();
                                if f.is_empty() { "Fuente: sin documento".to_string() }
                                else { format!("Fuente: {f}") }
                            }}
                        </span>
                        {move || (!ctx.filename.get().is_empty()).then(|| view! {
                            <span class="px-2 py-0.5 bg-[#b6d4fe]/30 text-[#436084] text-[9px] font-black rounded-sm">
                                "ES"
                            </span>
                        })}
                    </div>
                    <div class="flex gap-2 items-center">
                        // Botón para cargar nuevo documento
                        <button
                            class="text-[10px] font-bold uppercase tracking-tighter text-outline hover:text-primary flex items-center gap-1"
                            on:click=move |_| set_active_view.set(View::Dashboard)
                        >
                            <span class="material-symbols-outlined text-sm">"upload_file"</span>
                            "Cargar"
                        </button>
                    </div>
                </div>

                // Área de texto del documento
                // Si está vacío muestra editor; si tiene texto lo muestra como read-only
                {move || {
                    let text = ctx.text.get();
                    if text.is_empty() {
                        view! {
                            // ING-003: editor en blanco (textarea)
                            <textarea
                                class="flex-1 w-full p-12 font-serif text-lg leading-relaxed text-on-surf/90 resize-none border-none focus:ring-0 bg-white placeholder:text-slate-300"
                                placeholder="Escribe o pega el texto del documento aquí..."
                                on:input=move |ev| {
                                    ctx.text.set(event_target_value(&ev));
                                    ctx.filename.set("Documento sin título".to_string());
                                    ctx.word_count.set(
                                        ctx.text.get_untracked().split_whitespace().count() as u32
                                    );
                                }
                            />
                        }.into_any()
                    } else {
                        view! {
                            <div class="flex-1 overflow-y-auto p-12 font-serif text-base leading-relaxed text-on-surf/90 whitespace-pre-wrap">
                                {text}
                            </div>
                        }.into_any()
                    }
                }}

                // Pie del panel — estadísticas del documento
                <div class="h-10 bg-surf-low border-t border-slate-100 flex items-center justify-between px-6 shrink-0">
                    <div class="flex gap-4 text-[10px] font-bold uppercase text-outline">
                        <span>{move || format!("Palabras: {}", ctx.word_count.get())}</span>
                        <span>{move || {
                            let mins = (ctx.word_count.get() / 200).max(1);
                            format!("Lectura: ~{mins} min")
                        }}</span>
                    </div>
                    <div class="text-[10px] font-bold uppercase text-outline">"UTF-8"</div>
                </div>
            </section>

            // ── Column 2: Control Panel ────────────────────────────────────────
            // Parametrizes the transformation. All controls here feed into the
            // LLM prompt template system (§8.3 of SRS).
            //
            // TODO (Module 12 — PLT-001): "Action Selector" should be a proper
            // dropdown/accordion grouping all 18 SRS modules by category:
            //   Summarize       → Modules 2 (RES-001..RES-009)
            //   Communication   → Module 3 (GEN-001..GEN-011)
            //   Admin           → Module 4 (ADM-001..ADM-009)
            //   Analysis        → Modules 7,8,9,10
            //   Verifiability   → Modules 13,14
            <section class="w-[240px] bg-surf-low flex flex-col border-x border-slate-200/50">
                <div class="p-6 space-y-8 flex-1 overflow-y-auto">

                    // Action selector — todos los módulos SRS agrupados (RES, GEN, ADM, etc.)
                    <div>
                        <label class="block text-[10px] font-black uppercase tracking-widest text-on-surf-var mb-2">
                            "Action Selector"
                        </label>
                        <select
                            class="w-full bg-[#001b30] text-white border-none text-[11px] font-bold py-2 px-2 focus:ring-0 rounded"
                            on:change=move |ev| set_selected_action.set(event_target_value(&ev))
                        >
                            <optgroup label="── Resúmenes ──">
                                <option value="executive_summary" selected>"Resumen Ejecutivo"</option>
                                <option value="technical_summary">"Resumen Técnico"</option>
                                <option value="divulgative_summary">"Resumen Divulgativo"</option>
                                <option value="bullet_summary">"Puntos Clave"</option>
                                <option value="chronological_summary">"Resumen Cronológico"</option>
                                <option value="conclusions_summary">"Conclusiones y Recomendaciones"</option>
                                <option value="briefing_2min">"Briefing 2 min"</option>
                            </optgroup>
                            <optgroup label="── Comunicación ──">
                                <option value="press_release">"Nota de Prensa"</option>
                                <option value="headlines">"Titulares"</option>
                                <option value="linkedin_post">"Post LinkedIn"</option>
                                <option value="twitter_thread">"Hilo Twitter/X"</option>
                                <option value="blog_article">"Artículo de Blog"</option>
                                <option value="instagram_post">"Post Instagram"</option>
                                <option value="email_newsletter">"Email / Newsletter"</option>
                                <option value="speech">"Discurso"</option>
                                <option value="faqs">"FAQs"</option>
                                <option value="one_pager">"One-Pager / Ficha Resumen"</option>
                            </optgroup>
                            <optgroup label="── Administración ──">
                                <option value="key_quotes">"Citas Textuales"</option>
                                <option value="official_report">"Informe Oficial"</option>
                                <option value="meeting_minutes">"Acta de Reunión"</option>
                                <option value="administrative_resolution">"Resolución Administrativa"</option>
                                <option value="internal_memo">"Memorando Interno"</option>
                                <option value="allegations_response">"Alegaciones / Negociación"</option>
                            </optgroup>
                            <optgroup label="── Edición ──">
                                <option value="extract_commitments">"Compromisos Verificables"</option>
                                <option value="rewrite_formal">"Reescritura Formal"</option>
                                <option value="rewrite_shorter">"Reescritura Concisa"</option>
                                <option value="rewrite_persuasive">"Reescritura Persuasiva"</option>
                                <option value="rewrite_clearer">"Reescritura Clara"</option>
                                <option value="detect_redundancies">"Detectar Redundancias"</option>
                                <option value="translate_language">"Traducción"</option>
                                <option value="sentiment_analysis">"Análisis de Sentimiento"</option>
                                <option value="grammar_check">"Corrección Gramatical"</option>
                                <option value="simplify">"Simplificar (Lenguaje Claro)"</option>
                                <option value="detect_inconsistencies">"Detectar Inconsistencias"</option>
                                <option value="reformulate_paragraph">"Reformular Párrafo"</option>
                                <option value="detect_ambiguities">"Detectar Ambigüedades"</option>
                                <option value="improve_suggestions">"Sugerencias de Mejora"</option>
                                <option value="readability_analysis">"Análisis de Legibilidad"</option>
                                <option value="detect_evasive_language">"Lenguaje Evasivo"</option>
                            </optgroup>
                            <optgroup label="── Inteligencia ──">
                                <option value="semantic_versioning">"Versionado Semántico"</option>
                                <option value="merge_documents">"Fusión de Documentos"</option>
                                <option value="semantic_diff">"Diferencial Semántico"</option>
                                <option value="document_intersection">"Intersección Documental"</option>
                                <option value="detect_contradictions">"Detectar Contradicciones"</option>
                                <option value="versions_compare">"Comparar Versiones"</option>
                                <option value="inverse_questions">"Preguntas Inversas"</option>
                                <option value="press_release_check">"Verificar Nota de Prensa"</option>
                                <option value="validation_questions">"Checklist de Validación"</option>
                            </optgroup>
                            <optgroup label="── Extracción ──">
                                <option value="ner_extraction">"Entidades (NER)"</option>
                                <option value="keywords_extraction">"Palabras Clave"</option>
                                <option value="event_timeline">"Línea Temporal"</option>
                                <option value="impact_analysis">"Análisis de Impacto"</option>
                                <option value="verifiability_check">"Verificabilidad"</option>
                                <option value="evidence_gaps">"Huecos de Evidencia"</option>
                                <option value="traceability_map">"Mapa de Trazabilidad"</option>
                            </optgroup>
                            <optgroup label="── Privacidad ──">
                                <option value="anonymize">"Anonimización / Expurgo"</option>
                                <option value="preflight_check">"Preflight Documental"</option>
                                <option value="public_version">"Versión Pública"</option>
                                <option value="rgpd_check">"Verificación RGPD/LOPDGDD"</option>
                                <option value="style_linting">"Linting Documental"</option>
                                <option value="reader_simulation">"Simulador de Lector"</option>
                                <option value="generate_from_form">"Generar desde Formulario"</option>
                                <option value="generate_file_package">"Paquete de Expediente"</option>
                            </optgroup>
                            <optgroup label="── Crisis ──">
                                <option value="crisis_press_questions">"Simulacro Comparecencia"</option>
                                <option value="crisis_communication">"Kit de Crisis Reputacional"</option>
                                <option value="argumentario">"Argumentario"</option>
                                <option value="difficult_questions_simulator">"Simulador Preguntas Difíciles"</option>
                            </optgroup>
                        </select>
                    </div>

                    // Output length slider (RES-007: 50–500 words)
                    <div class="space-y-6">
                        <div>
                            <div class="flex justify-between mb-2">
                                <label class="text-[10px] font-black uppercase text-on-surf-var">"Length"</label>
                                <span class="text-[10px] font-bold text-primary">
                                    {move || format!("{}w", length_words.get())}
                                </span>
                            </div>
                            <input
                                class="w-full accent-primary"
                                type="range" min="50" max="500"
                                prop:value={move || length_words.get()}
                                style="height:2px; background:#e1e3e4;"
                                on:input=move |ev| {
                                    if let Ok(v) = event_target_value(&ev).parse::<u32>() {
                                        set_length.set(v);
                                    }
                                }
                            />
                        </div>

                        // Tone slider (TON-001: 1=Coloquial … 5=Formal Institucional)
                        <div>
                            <div class="flex justify-between mb-2">
                                <label class="text-[10px] font-black uppercase text-on-surf-var">"Tone"</label>
                                <span class="text-[10px] font-bold text-primary">
                                    {move || tone_label(tone.get())}
                                </span>
                            </div>
                            <input
                                class="w-full accent-primary"
                                type="range" min="1" max="5"
                                prop:value={move || tone.get()}
                                style="height:2px; background:#e1e3e4;"
                                on:input=move |ev| {
                                    if let Ok(v) = event_target_value(&ev).parse::<u32>() {
                                        set_tone.set(v);
                                    }
                                }
                            />
                        </div>

                        // Audience selector (RES-008) — valor inyectado en {publico_objetivo}
                        <div>
                            <label class="block text-[10px] font-black uppercase text-on-surf-var mb-2">
                                "Audience"
                            </label>
                            <select
                                class="w-full bg-surf-highest border-none text-[11px] font-bold py-2 focus:ring-0 rounded"
                                on:change=move |ev| set_audience.set(event_target_value(&ev))
                            >
                                <option value="citizen">"Ciudad / Ciudadanía"</option>
                                <option value="press">"Prensa / Global"</option>
                                <option value="technical" selected>"Técnico / Interno"</option>
                                <option value="executive">"Ejecutivo / Decisor"</option>
                            </select>
                        </div>
                    </div>
                </div>

                // Generate button — CTA principal del editor (§8.3 motor de prompts)
                <div class="p-4 bg-[#001b30]">
                    <button
                        class="w-full bg-[#2E75B6] hover:bg-[#66a6ea] active:scale-[0.98] transition-all text-white py-4 rounded-sm flex flex-col items-center justify-center gap-1 group shadow-[0_0_20px_rgba(46,117,182,0.3)] disabled:opacity-40 disabled:cursor-not-allowed"
                        disabled=move || ctx.processing.get() || ctx.text.get().is_empty()
                        on:click=move |_| {
                            run_transform(
                                ctx,
                                selected_action.get_untracked(),
                                length_words.get_untracked(),
                                tone.get_untracked(),
                                audience.get_untracked(),
                                "es".to_string(),
                            );
                        }
                    >
                        <span class="material-symbols-outlined text-2xl group-active:animate-pulse">"bolt"</span>
                        <span class="text-[11px] font-black uppercase tracking-widest">
                            {move || if ctx.processing.get() { "Procesando..." } else { "Generate" }}
                        </span>
                    </button>
                    <p class="text-[8px] text-center mt-3 text-slate-500 font-bold uppercase tracking-tight">
                        {move || {
                            let t = ctx.text.get();
                            if t.is_empty() { "Carga un documento para empezar".to_string() }
                            else { format!("Modelo: Qwen 3.5 · {} palabras", t.split_whitespace().count()) }
                        }}
                    </p>
                </div>
            </section>

            // ── Column 3: Result Area ──────────────────────────────────────────
            // Muestra el output del LLM token a token (SSE streaming).
            // Borde naranja 4px = "output procesado / refinado".
            <section class="flex-1 flex flex-col border-l-[4px] border-[#C45911] bg-surface">
                // Cabecera del resultado
                <div class="h-12 border-b border-slate-100 flex items-center justify-between px-6 bg-surf-highest/30">
                    <div class="flex items-center gap-4">
                        <span class="text-[10px] font-bold uppercase tracking-tighter text-[#C45911]">
                            {move || {
                                let lbl = ctx.output_label.get();
                                if lbl.is_empty() { "Output".to_string() }
                                else { format!("Output: {lbl}") }
                            }}
                        </span>
                        // Cursor parpadeante mientras streameamos
                        {move || ctx.processing.get().then(|| view! {
                            <span class="w-2 h-4 bg-[#C45911] animate-pulse inline-block"></span>
                        })}
                    </div>
                    <div class="flex gap-4">
                        // EXO-001: copiar al portapapeles
                        <button
                            class="flex items-center gap-1 text-[10px] font-bold uppercase text-outline hover:text-primary transition-colors disabled:opacity-30"
                            disabled=move || !has_output()
                            on:click=move |_| copy_to_clipboard(ctx.output.get_untracked())
                        >
                            <span class="material-symbols-outlined text-sm">"content_copy"</span>
                            " Copy"
                        </button>
                        // EXO-002..EXO-005: modal de exportación
                        <button
                            class="flex items-center gap-1 text-[10px] font-bold uppercase text-outline hover:text-primary transition-colors disabled:opacity-30"
                            disabled=move || !has_output()
                            on:click=move |_| show_export.set(true)
                        >
                            <span class="material-symbols-outlined text-sm">"ios_share"</span>
                            " Export"
                        </button>
                    </div>
                </div>

                // Área de texto generado
                <div class="flex-1 p-12 overflow-y-auto">
                    {move || if has_output() {
                        view! {
                            <div class="max-w-2xl">
                                <div class="mb-8 inline-block px-3 py-1 bg-[#ffdbcb] text-[#783100] text-[9px] font-black uppercase tracking-widest rounded-sm">
                                    {move || ctx.output_label.get()}
                                </div>
                                // Texto generado — whitespace-pre-wrap preserva saltos de línea del LLM
                                <div class="font-serif text-lg text-on-surf/90 leading-relaxed whitespace-pre-wrap">
                                    {move || ctx.output.get()}
                                </div>
                            </div>
                        }.into_any()
                    } else {
                        view! {
                            // Empty state — antes de la primera generación
                            <div class="flex flex-col items-center justify-center h-full text-center opacity-40">
                                <span class="material-symbols-outlined text-[48px] text-primary mb-4">"bolt"</span>
                                <p class="font-serif italic text-xl text-primary">
                                    "Configura los parámetros y pulsa Generate"
                                </p>
                            </div>
                        }.into_any()
                    }}
                </div>

                // Pie del resultado — metadatos de procedencia (TRA-001)
                {move || has_output().then(|| view! {
                    <div class="p-6 bg-surf-low/50 border-t border-slate-100">
                        <div class="flex flex-wrap items-center gap-x-6 gap-y-2 opacity-60">
                            <div class="flex items-center gap-2">
                                <span class="material-symbols-outlined text-xs">"verified"</span>
                                <span class="text-[10px] font-bold uppercase tracking-tight">
                                    {move || format!("Generated: {}", ctx.output_label.get())}
                                </span>
                            </div>
                            <span class="w-1 h-1 bg-outline rounded-full"></span>
                            <span class="text-[10px] font-bold uppercase tracking-tight">
                                {move || format!("{} palabras", ctx.output.get().split_whitespace().count())}
                            </span>
                            <span class="w-1 h-1 bg-outline rounded-full"></span>
                            <span class="text-[10px] font-bold uppercase tracking-tight">"Qwen 3.5 · Local"</span>
                        </div>
                        <div class="mt-6 flex gap-3">
                            <button
                                class="border border-outline-var text-on-surf text-[10px] font-black uppercase px-6 py-2.5 hover:bg-surf-high transition-colors flex items-center gap-2"
                                on:click=move |_| {
                                    run_transform(
                                        ctx,
                                        selected_action.get_untracked(),
                                        length_words.get_untracked(),
                                        tone.get_untracked(),
                                        audience.get_untracked(),
                                        "es".to_string(),
                                    );
                                }
                            >
                                <span class="material-symbols-outlined text-sm">"refresh"</span>
                                " Regenerate"
                            </button>
                        </div>
                    </div>
                })}
            </section>

            // Modal de exportación (EXO-001..EXO-006)
            <ExportModal show=show_export ctx=ctx toast=save_toast/>

            // Toast de confirmación de guardado
            {move || save_toast.get().map(|msg| view! {
                <div class="fixed bottom-6 left-1/2 -translate-x-1/2 z-[300] pointer-events-none
                            bg-[#002542] text-white text-[12px] font-semibold px-5 py-3
                            rounded-full shadow-2xl border border-[#2E75B6] flex items-center gap-2">
                    <span class="material-symbols-outlined text-[#C45911] text-[16px]">"check_circle"</span>
                    {msg}
                </div>
            })}

            // Floating status ribbon — anchored bottom-right
            <div class="fixed bottom-6 right-6 z-50 pointer-events-none">
                <div class="bg-[#002542] text-[#66a6ea] px-4 py-2 border-l-4 border-[#C45911] flex items-center gap-4 shadow-2xl">
                    <div class="flex gap-1">
                        <div class="w-1 h-3 bg-white/20"></div>
                        <div class="w-1 h-3 bg-[#66a6ea]"></div>
                        <div class="w-1 h-3 bg-[#66a6ea]"></div>
                    </div>
                    <span class="text-[10px] font-black uppercase tracking-[0.2em]">
                        "Processing Secured: Local Node 04"
                    </span>
                </div>
            </div>
        </div>
    }
}

// ─── Modal de exportación (EXO-001..EXO-006) ─────────────────────────────────

#[component]
fn ExportModal(
    show:  RwSignal<bool>,
    ctx:   DocumentCtx,
    toast: RwSignal<Option<String>>,
) -> impl IntoView {
    view! {
        {move || show.get().then(|| {
            let label  = ctx.output_label.get_untracked();
            let output = ctx.output.get_untracked();
            let fname  = ctx.filename.get_untracked()
                .trim_end_matches(".pdf")
                .trim_end_matches(".docx")
                .trim_end_matches(".txt")
                .to_string();
            let base = if fname.is_empty() { "output".to_string() } else { fname };

            view! {
                <div
                    class="fixed inset-0 z-[200] flex items-end justify-end pb-28 pr-8"
                    on:click=move |_| show.set(false)
                >
                    <div
                        class="bg-white rounded-xl shadow-2xl border border-slate-200 w-72 overflow-hidden"
                        on:click=|ev| ev.stop_propagation()
                    >
                        <div class="px-5 py-4 border-b border-slate-100 flex justify-between items-center">
                            <span class="text-[10px] font-black uppercase tracking-widest text-primary">
                                "Exportar resultado"
                            </span>
                            <button class="text-slate-400 hover:text-primary"
                                on:click=move |_| show.set(false)>
                                <span class="material-symbols-outlined text-sm">"close"</span>
                            </button>
                        </div>
                        <div class="p-3 space-y-1">
                            // EXO-001: copiar al portapapeles
                            {let (o, s) = (output.clone(), show);
                            view! {
                                <ExportRow icon="content_copy" label="Copiar al portapapeles"
                                    on_click=move || { copy_to_clipboard(o.clone()); s.set(false); }
                                />
                            }}
                            // EXO-005: Markdown (blob download — funciona para texto)
                            {let (o, s, b, l) = (output.clone(), show, base.clone(), label.clone());
                            view! {
                                <ExportRow icon="code" label="Markdown (.md)"
                                    on_click=move || {
                                        download_text(o.clone(), &format!("{b} - {l}.md"), "text/markdown");
                                        s.set(false);
                                    }
                                />
                            }}
                            // EXO-003: Texto plano (blob download)
                            {let (o, s, b, l) = (output.clone(), show, base.clone(), label.clone());
                            view! {
                                <ExportRow icon="text_snippet" label="Texto plano (.txt)"
                                    on_click=move || {
                                        download_text(o.clone(), &format!("{b} - {l}.txt"), "text/plain");
                                        s.set(false);
                                    }
                                />
                            }}
                            // EXO-002: DOCX — guardado en workspace + Finder
                            {let (o, s, b, l, t) = (output.clone(), show, base.clone(), label.clone(), toast);
                            view! {
                                <ExportRow icon="description" label="Word Document (.docx)"
                                    on_click=move || {
                                        let (text, lbl, fname) = (o.clone(), l.clone(), format!("{b} - {l}.docx"));
                                        s.set(false);
                                        spawn_local(async move {
                                            fetch_render(text, lbl, "docx".to_string(), fname, t).await;
                                        });
                                    }
                                />
                            }}
                            // EXO-006: PDF — guardado en workspace + Finder
                            {let (o, s, b, l, t) = (output.clone(), show, base.clone(), label.clone(), toast);
                            view! {
                                <ExportRow icon="picture_as_pdf" label="PDF Document (.pdf)"
                                    on_click=move || {
                                        let (text, lbl, fname) = (o.clone(), l.clone(), format!("{b} - {l}.pdf"));
                                        s.set(false);
                                        spawn_local(async move {
                                            fetch_render(text, lbl, "pdf".to_string(), fname, t).await;
                                        });
                                    }
                                />
                            }}
                        </div>
                    </div>
                </div>
            }
        })}
    }
}

#[component]
fn ExportRow(icon: &'static str, label: &'static str, on_click: impl Fn() + 'static) -> impl IntoView {
    view! {
        <button
            class="w-full flex items-center gap-3 px-4 py-3 rounded-lg hover:bg-surf-low transition-colors text-left group"
            on:click=move |_| on_click()
        >
            <span class="material-symbols-outlined text-[18px] text-outline group-hover:text-primary">{icon}</span>
            <span class="text-[11px] font-bold text-on-surf group-hover:text-primary">{label}</span>
        </button>
    }
}

#[component]
fn ControlMenuBtn(label: &'static str) -> impl IntoView {
    view! {
        // TODO: add on:click handler when action selection is wired to state
        <button class="w-full text-left px-3 py-1.5 text-[11px] font-medium text-outline hover:bg-surf-highest transition-colors">
            {label}
        </button>
    }
}

#[component]
fn BulletItem(text: &'static str) -> impl IntoView {
    view! {
        <li class="flex items-start gap-3">
            <span class="w-1.5 h-1.5 bg-[#C45911] mt-1.5 shrink-0"></span>
            {text}
        </li>
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// VIEW: ANALYSIS (Document Analysis Dashboard)
// ═══════════════════════════════════════════════════════════════════════════════
// Forensic intelligence report for the currently loaded document.
// Uses an asymmetric bento grid to present multiple analytical dimensions
// simultaneously — readable at a glance, drillable on click.
//
// Corresponds to SRS modules:
//   7  (Forensic Text Analysis — FOR-001..FOR-005)
//   8  (Textual Arithmetic — ARI-001..ARI-006)
//   9  (Inverse Questions — INV-001..INV-005)
//   10 (Structured Extraction and Metadata — EXT-001..EXT-007)
//   5  (Tone / Sentiment — TON-005, TON-006)
//   13 (Verifiability — VER-001..VER-005)
//
// TODO: Trigger a full analysis run on document load via POST /api/analyze.
// The server runs all analysis modules in parallel (goroutines / async tasks)
// and returns a structured AnalysisReport JSON. Store in SQLite for the audit log.
// All values shown here are currently static mockups.

// ─── Helper: SHA-256 de un string vía Web Crypto API ─────────────────────────
// Devuelve el hash como string hexadecimal en minúsculas (64 caracteres).
// Se usa como clave de caché en /api/analysis.

async fn sha256_hex(text: &str) -> Option<String> {
    use js_sys::{ArrayBuffer, Uint8Array};
    let window = web_sys::window()?;
    let crypto = window.crypto().ok()?;
    let subtle = crypto.subtle();
    // Convertir el string a Vec<u8> UTF-8
    let mut bytes: Vec<u8> = text.as_bytes().to_vec();
    let promise = subtle.digest_with_str_and_u8_array("SHA-256", &mut bytes).ok()?;
    let result = JsFuture::from(promise).await.ok()?;
    let ab: ArrayBuffer = result.unchecked_into();
    let hash_bytes = Uint8Array::new(&ab).to_vec();
    Some(hash_bytes.iter().map(|b| format!("{:02x}", b)).collect())
}

// ─── AnalysisResult: estado parseado del análisis ─────────────────────────────

#[derive(Clone, Default)]
struct AnalysisResult {
    // FOR-001: legibilidad
    readability_raw:  String,   // texto libre del LLM
    // TON-005 / TON-006: sentimiento
    sentiment_raw:    String,
    // FOR-002 / INV-*: anomalías (texto libre del LLM)
    anomalies_raw:    String,
    // EXT-001: entidades NER (texto libre, parseamos líneas)
    ner_raw:          String,
    // EXT-003 / EXT-004 / EXT-005: metadatos
    keywords_raw:     String,
    // EXT-006: timeline
    timeline_raw:     String,
    // EXT-007: impacto
    impact_raw:       String,
}

// Acumula la respuesta SSE de una acción de análisis y devuelve el texto completo.
// Usa Rc<RefCell<String>> para acumular sin necesitar RwSignal fuera del componente.
async fn collect_action(text: String, action: &'static str) -> String {
    use std::rc::Rc;
    use std::cell::RefCell;
    let buf = Rc::new(RefCell::new(String::new()));
    let body = serde_json::json!({
        "text":         text,
        "action":       action,
        "doc_name":     "",
        "length_words": 300u32,
        "tone":         "4",
        "audience":     "técnico",
        "language":     "es",
    }).to_string();
    let headers = web_sys::Headers::new().unwrap();
    headers.set("Content-Type", "application/json").unwrap();
    let opts = web_sys::RequestInit::new();
    opts.set_method("POST");
    opts.set_body(&wasm_bindgen::JsValue::from_str(&body));
    opts.set_headers(&wasm_bindgen::JsValue::from(headers));
    let req = web_sys::Request::new_with_str_and_init("/api/transform", &opts).unwrap();
    let window = web_sys::window().unwrap();
    let resp: web_sys::Response = match JsFuture::from(window.fetch_with_request(&req)).await {
        Ok(r)  => r.unchecked_into(),
        Err(_) => return String::new(),
    };
    let buf2 = buf.clone();
    read_sse_stream(
        resp,
        move |t| buf2.borrow_mut().push_str(&t),
        || {},
    ).await;
    let result = buf.borrow().clone();
    result
}

// ─── Función libre: ejecutar análisis con caché ───────────────────────────────
// Se llama desde dos closures distintas (run_analysis / reanalyze) sin problema
// de ownership porque todos los parámetros son Copy (RwSignal) o se pasan owned.

#[allow(clippy::too_many_arguments)]
async fn do_analysis(
    text:         String,
    hash:         String,
    force:        bool,
    analyzing:    RwSignal<bool>,
    result:       RwSignal<Option<AnalysisResult>>,
    from_cache:   RwSignal<bool>,
    cached_at:    RwSignal<String>,
    current_step: RwSignal<&'static str>,
    ctx:          DocumentCtx,
) {
    analyzing.set(true);
    result.set(None);
    from_cache.set(false);
    cached_at.set(String::new());

    // ── 1. Consultar caché en oliv_analysis_cache (salvo que force=true) ────────
    if !force && !hash.is_empty() {
        current_step.set("Consultando caché…");
        let rows = plugin_query(
            "SELECT readability_raw, sentiment_raw, anomalies_raw, ner_raw, \
             keywords_raw, timeline_raw, impact_raw, created_at \
             FROM oliv_analysis_cache WHERE doc_hash = ?1",
            vec![serde_json::json!(hash)],
        ).await;
        if let Some(v) = rows.into_iter().next() {
            let r = AnalysisResult {
                readability_raw: v["readability_raw"].as_str().unwrap_or("").to_string(),
                sentiment_raw:   v["sentiment_raw"].as_str().unwrap_or("").to_string(),
                anomalies_raw:   v["anomalies_raw"].as_str().unwrap_or("").to_string(),
                ner_raw:         v["ner_raw"].as_str().unwrap_or("").to_string(),
                keywords_raw:    v["keywords_raw"].as_str().unwrap_or("").to_string(),
                timeline_raw:    v["timeline_raw"].as_str().unwrap_or("").to_string(),
                impact_raw:      v["impact_raw"].as_str().unwrap_or("").to_string(),
            };
            let ts = v["created_at"].as_str().unwrap_or("").to_string();
            current_step.set("");
            from_cache.set(true);
            cached_at.set(ts);
            result.set(Some(r));
            analyzing.set(false);
            return; // ← Hit: salimos sin llamar al LLM
        }
        // Miss → seguimos al análisis LLM
    }

    // ══════════════════════════════════════════════════════════════════════════
    // TODO — ARQUITECTURA HÍBRIDA NLP/BERT (Fase futura)
    //
    // El pipeline actual delega todos los módulos al LLM local (Qwen 3.5).
    // A medida que el producto madure, cada módulo debería usar la herramienta
    // óptima para su tarea — no siempre un LLM completo:
    //
    // ── CÁLCULO PURO EN RUST (sin modelo) ────────────────────────────────────
    //
    //   FOR-001 · Legibilidad
    //     Implementar directamente en `server/src/tools/` el índice
    //     Flesch-Szigriszt (fórmula pública para español) sobre el texto fuente:
    //       score = 206.835 - 62.3*(sílabas/palabras) - (palabras/frases)
    //     También: longitud media de frase, densidad léxica (tipos/tokens).
    //     Ventaja: resultado instantáneo (<1ms), determinista, sin LLM.
    //     Podría mostrarse en tiempo real mientras el usuario edita en el Editor.
    //     Crate sugerido: ninguno — implementación manual de ~50 líneas.
    //
    // ── MODELOS BERT/TRANSFORMER PEQUEÑOS vía `candle` ───────────────────────
    //
    //   `candle` (HuggingFace) es un runtime de inferencia en Rust puro que
    //   carga modelos SafeTensors/GGUF sin Python, sin ONNX Runtime, compatible
    //   con el requisito air-gap del SRS. Añadir al Cargo.toml del servidor:
    //     candle-core = "0.9"
    //     candle-transformers = "0.9"
    //
    //   EXT-001 · NER (Extracción de Entidades)
    //     Modelo recomendado: PlanTL-GOB-ES/roberta-base-bsc-ner (español)
    //     o bert-base-multilingual-cased fine-tuned para NER.
    //     Tamaño: ~250–500MB. Latencia en CPU: <100ms por documento.
    //     Ventaja sobre LLM: precisión superior en clasificación de entidades
    //     (PERSON, ORG, DATE, LOC, AMOUNT), salida estructurada JSON directa.
    //     El LLM actual da texto libre que hay que parsear.
    //
    //   TON-005 · Sentimiento y tono emocional
    //     Modelo recomendado: pysentimiento/robertuito-sentiment-analysis
    //     (fine-tuned para español, 3 clases: POS/NEG/NEU).
    //     Para la escala Hostile→Neutral→Advocacy se puede entrenar un
    //     clasificador de 5 clases sobre corpus institucional propio.
    //     Latencia en CPU: <50ms.
    //
    //   FOR-002 · Detección de lenguaje evasivo
    //     Un clasificador de secuencias fine-tuned sobre ejemplos de lenguaje
    //     pasivo/evasivo institucional. Alternativa más ligera: reglas NLP
    //     con análisis de voz pasiva vía PoS tagging (crate `rust-stemmers`
    //     + diccionario de patrones).
    //
    // ── LLM LOCAL (mantener para tareas de razonamiento) ─────────────────────
    //
    //   INV-001..005 · Anomalías y preguntas inversas
    //     Requiere comprensión contextual profunda → LLM.
    //     No hay modelo pequeño que detecte "compromiso sin criterio de
    //     cumplimiento" o "cifra sin fuente" con la misma fiabilidad.
    //
    //   EXT-007 · Análisis de impacto
    //     Requiere razonamiento sobre consecuencias → LLM.
    //
    //   EXT-006 · Línea temporal
    //     Podría migrarse a NER (extracción de DATE) + ordenación,
    //     pero la narrativización ("este evento provocó...") necesita LLM.
    //
    //   EXT-003 · Palabras clave y metadatos
    //     Candidato para TF-IDF clásico (sin modelo) o KeyBERT.
    //     KeyBERT usa embeddings de sentence-transformers para extraer
    //     keywords semánticamente relevantes. Crate: `fastembed` en Rust.
    //
    // ── PLAN DE MIGRACIÓN SUGERIDO ────────────────────────────────────────────
    //
    //   Fase 1: Flesch-Szigriszt en Rust puro (FOR-001) — trivial, alto impacto
    //   Fase 2: NER con candle+roberta-bsc (EXT-001) — mayor ganancia vs LLM
    //   Fase 3: Sentimiento con robertuito (TON-005) — rápido de integrar
    //   Fase 4: Keywords con fastembed TF-IDF (EXT-003) — eliminar 1 LLM call
    //   Fase 5: Lenguaje evasivo con PoS rules (FOR-002) — determinista
    //   Resto:  INV-*, EXT-007 permanecen en LLM (razonamiento complejo)
    //
    //   Al final del plan: de 7 llamadas LLM → 2-3 llamadas LLM + 4-5 módulos
    //   instantáneos. Tiempo de análisis estimado: de ~3min → ~20s.
    //
    // ══════════════════════════════════════════════════════════════════════════

    // ── 2. Análisis LLM completo (7 módulos secuenciales) ────────────────────
    let mut r = AnalysisResult::default();

    current_step.set("Legibilidad (FOR-001)");
    // TODO Fase 1: reemplazar por cálculo Flesch-Szigriszt en Rust puro (server/src/tools/readability.rs)
    r.readability_raw = collect_action(text.clone(), "readability_analysis").await;

    current_step.set("Sentimiento (TON-005)");
    // TODO Fase 3: reemplazar por pysentimiento/robertuito vía candle
    r.sentiment_raw = collect_action(text.clone(), "sentiment_analysis").await;

    current_step.set("Anomalías / Preguntas Inversas (INV-001..005)");
    // Mantener en LLM — requiere razonamiento contextual profundo
    r.anomalies_raw = collect_action(text.clone(), "inverse_questions").await;

    current_step.set("Entidades NER (EXT-001)");
    // TODO Fase 2: reemplazar por PlanTL-GOB-ES/roberta-base-bsc-ner vía candle
    r.ner_raw = collect_action(text.clone(), "ner_extraction").await;

    current_step.set("Palabras clave y metadatos (EXT-003)");
    // TODO Fase 4: reemplazar por TF-IDF o fastembed KeyBERT en Rust
    r.keywords_raw = collect_action(text.clone(), "keywords_extraction").await;

    current_step.set("Línea temporal (EXT-006)");
    // TODO Fase 2 (parcial): extracción DATE vía NER, narrativización conserva LLM
    r.timeline_raw = collect_action(text.clone(), "event_timeline").await;

    current_step.set("Análisis de impacto (EXT-007)");
    // Mantener en LLM — razonamiento sobre consecuencias
    r.impact_raw = collect_action(text.clone(), "impact_analysis").await;

    // ── 3. Guardar en oliv_analysis_cache y actualizar oliv_projects ────────────
    if !hash.is_empty() {
        current_step.set("Guardando en base de datos…");
        // Guardar análisis
        plugin_query(
            "INSERT OR REPLACE INTO oliv_analysis_cache \
             (doc_hash, doc_name, word_count, readability_raw, sentiment_raw, \
              anomalies_raw, ner_raw, keywords_raw, timeline_raw, impact_raw) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            vec![
                serde_json::json!(hash),
                serde_json::json!(ctx.filename.get_untracked()),
                serde_json::json!(ctx.word_count.get_untracked()),
                serde_json::json!(r.readability_raw),
                serde_json::json!(r.sentiment_raw),
                serde_json::json!(r.anomalies_raw),
                serde_json::json!(r.ner_raw),
                serde_json::json!(r.keywords_raw),
                serde_json::json!(r.timeline_raw),
                serde_json::json!(r.impact_raw),
            ],
        ).await;
        // Marcar proyecto como analizado
        plugin_query(
            "UPDATE oliv_projects SET has_analysis = 1, \
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ','now') \
             WHERE doc_hash = ?1",
            vec![serde_json::json!(hash)],
        ).await;
    }

    current_step.set("");
    result.set(Some(r));
    analyzing.set(false);
}

#[component]
fn AnalysisView() -> impl IntoView {
    let ctx = use_context::<DocumentCtx>().expect("DocumentCtx");

    // Estado reactivo del informe de análisis
    let result:       RwSignal<Option<AnalysisResult>> = RwSignal::new(None);
    let analyzing:    RwSignal<bool>                   = RwSignal::new(false);
    let current_step: RwSignal<&'static str>           = RwSignal::new("");
    let from_cache:   RwSignal<bool>                   = RwSignal::new(false);
    let cached_at:    RwSignal<String>                 = RwSignal::new(String::new());
    // Toast de exportación
    let toast:        RwSignal<Option<String>>          = RwSignal::new(None);

    // ── Botón "Ejecutar Análisis" (con caché) ────────────────────────────────
    // Usa ctx.doc_hash si ya lo calculó el servidor en /api/extract;
    // si está vacío (texto pegado manualmente) lo calcula aquí en WASM.
    let run_analysis = move |_| {
        let text = ctx.text.get_untracked();
        if text.is_empty() { return; }
        let hash_cached = ctx.doc_hash.get_untracked();
        spawn_local(async move {
            let hash = if hash_cached.is_empty() {
                sha256_hex(&text).await.unwrap_or_default()
            } else { hash_cached };
            do_analysis(text, hash, false, analyzing, result, from_cache, cached_at, current_step, ctx).await;
        });
    };

    // ── Botón "Re-analizar" (fuerza LLM, ignora caché) ───────────────────────
    let reanalyze = move |_| {
        let text = ctx.text.get_untracked();
        if text.is_empty() { return; }
        let hash_cached = ctx.doc_hash.get_untracked();
        spawn_local(async move {
            let hash = if hash_cached.is_empty() {
                sha256_hex(&text).await.unwrap_or_default()
            } else { hash_cached };
            do_analysis(text, hash, true, analyzing, result, from_cache, cached_at, current_step, ctx).await;
        });
    };

    // ── Exportar informe completo como texto ──────────────────────────────────
    let export_report = move |_| {
        if let Some(r) = result.get_untracked() {
            let fname = ctx.filename.get_untracked();
            let label = format!("Análisis — {}", if fname.is_empty() { "documento" } else { &fname });
            let text = format!(
                "# {}\n\n## Legibilidad\n{}\n\n## Sentimiento y Tono\n{}\n\n## Anomalías\n{}\n\n## Entidades (NER)\n{}\n\n## Palabras Clave\n{}\n\n## Línea Temporal\n{}\n\n## Análisis de Impacto\n{}",
                label,
                r.readability_raw, r.sentiment_raw, r.anomalies_raw,
                r.ner_raw, r.keywords_raw, r.timeline_raw, r.impact_raw
            );
            spawn_local(async move {
                fetch_render(text, label, "txt".to_string(), "analysis.txt".to_string(), toast).await;
            });
        }
    };

    let has_doc   = move || !ctx.text.get().is_empty();
    let has_result = move || result.get().is_some();

    view! {
        <div class="p-10 max-w-7xl mx-auto">

            // ── Toast de exportación ───────────────────────────────────────────
            {move || toast.get().map(|msg| view! {
                <div class="fixed bottom-6 right-6 z-50 bg-primary text-[#66a6ea] px-5 py-3 rounded-sm shadow-2xl flex items-center gap-3">
                    <span class="material-symbols-outlined text-[18px]">"check_circle"</span>
                    <span class="text-[11px] font-bold uppercase tracking-widest">{msg}</span>
                </div>
            })}

            // ── Page Header ────────────────────────────────────────────────────
            <header class="mb-8">
                <div class="flex justify-between items-end flex-wrap gap-4">
                    <div>
                        <h2 class="text-4xl font-sans font-black tracking-tighter text-primary uppercase">
                            "Análisis Documental"
                        </h2>
                        <p class="font-serif italic text-xl text-outline mt-1">
                            {move || {
                                let f = ctx.filename.get();
                                if f.is_empty() {
                                    "Sin documento cargado — carga uno desde el Editor".to_string()
                                } else {
                                    format!("Documento: {f}")
                                }
                            }}
                        </p>
                    </div>
                    <div class="flex items-center gap-3">
                        // Badge offline
                        <div class="flex items-center gap-2 px-3 py-2 bg-primary text-[#66a6ea] rounded-sm">
                            <span class="material-symbols-outlined text-[18px]">"lock"</span>
                            <span class="text-[10px] font-bold uppercase tracking-[0.2em]">"100% Offline"</span>
                        </div>
                        // Badge "Desde caché" + botón Re-analizar
                        {move || (has_result() && from_cache.get()).then(|| view! {
                            <div class="flex items-center gap-2">
                                <div class="flex items-center gap-2 px-3 py-2 bg-[#003b65] text-[#66a6ea] rounded-sm">
                                    <span class="material-symbols-outlined text-[16px]">"database"</span>
                                    <div>
                                        <span class="block text-[10px] font-black uppercase tracking-widest">"Desde BD"</span>
                                        <span class="block text-[9px] text-[#9ecaff]">
                                            {move || {
                                                let ts = cached_at.get();
                                                if ts.len() >= 10 { ts[..10].to_string() } else { ts }
                                            }}
                                        </span>
                                    </div>
                                </div>
                                <button
                                    on:click=reanalyze
                                    class="px-3 py-2 text-[10px] font-bold uppercase tracking-widest text-outline border border-outline/40 rounded-sm hover:border-primary hover:text-primary transition-all"
                                    title="Forzar nuevo análisis ignorando la caché"
                                >
                                    "Re-analizar"
                                </button>
                            </div>
                        })}
                        // Botón exportar (sólo si hay resultado)
                        {move || has_result().then(|| view! {
                            <button
                                on:click=export_report
                                class="px-4 py-2 text-[11px] font-bold uppercase tracking-widest text-primary border border-primary rounded-sm hover:bg-primary hover:text-white transition-all"
                            >
                                "Exportar Informe"
                            </button>
                        })}
                        // Botón principal
                        {move || {
                            let disabled = !has_doc() || analyzing.get();
                            view! {
                                <button
                                    on:click=run_analysis
                                    disabled=disabled
                                    class=move || format!(
                                        "px-6 py-2 text-[11px] font-bold uppercase tracking-widest rounded-sm transition-all {}",
                                        if disabled {
                                            "bg-surface-container text-outline cursor-not-allowed"
                                        } else {
                                            "bg-[#C45911] text-white hover:opacity-90 active:scale-95 shadow-md"
                                        }
                                    )
                                >
                                    {move || if analyzing.get() { "Analizando..." } else { "Ejecutar Análisis" }}
                                </button>
                            }
                        }}
                    </div>
                </div>
            </header>

            // ── Barra de progreso del análisis ────────────────────────────────
            {move || analyzing.get().then(|| view! {
                <div class="mb-8 p-4 bg-[#001b30] rounded-sm border border-[#003b65]">
                    <div class="flex items-center gap-3">
                        <div class="w-2 h-2 rounded-full bg-[#66a6ea] animate-pulse shrink-0"></div>
                        <span class="text-[11px] font-bold uppercase tracking-widest text-[#66a6ea]">
                            "Motor IA activo — "
                        </span>
                        <span class="text-[11px] text-[#9ecaff]">
                            {move || current_step.get()}
                        </span>
                    </div>
                    <div class="mt-3 h-1 bg-[#003b65] rounded-full overflow-hidden">
                        <div class="h-full bg-[#66a6ea] animate-pulse" style="width:60%"></div>
                    </div>
                </div>
            })}

            // ── Empty state: sin documento ────────────────────────────────────
            {move || (!has_doc() && !analyzing.get()).then(|| view! {
                <div class="flex flex-col items-center justify-center py-32 text-center">
                    <span class="material-symbols-outlined text-6xl text-outline/30 mb-6">"description"</span>
                    <h3 class="font-sans font-black text-xl text-primary mb-2">"Sin documento cargado"</h3>
                    <p class="font-serif italic text-outline max-w-md">
                        "Carga un documento desde la vista Editor para poder ejecutar el análisis forense completo."
                    </p>
                </div>
            })}

            // ── Empty state: doc cargado pero sin analizar ─────────────────────
            {move || (has_doc() && !has_result() && !analyzing.get()).then(|| view! {
                <div class="flex flex-col items-center justify-center py-24 text-center">
                    <span class="material-symbols-outlined text-5xl text-outline/30 mb-6">"analytics"</span>
                    <h3 class="font-sans font-black text-xl text-primary mb-2">"Listo para analizar"</h3>
                    <p class="font-serif italic text-outline max-w-md mb-6">
                        "Pulsa «Ejecutar Análisis» para obtener el informe forense completo: legibilidad, sentimiento, entidades, anomalías, metadatos, timeline e impacto."
                    </p>
                    <div class="grid grid-cols-2 md:grid-cols-4 gap-4 text-left max-w-2xl">
                        <AnalysisModuleBadge icon="menu_book"     label="FOR-001" desc="Legibilidad"/>
                        <AnalysisModuleBadge icon="psychology"    label="TON-005" desc="Sentimiento"/>
                        <AnalysisModuleBadge icon="report"        label="INV-001" desc="Anomalías"/>
                        <AnalysisModuleBadge icon="manage_search" label="EXT-001" desc="NER"/>
                        <AnalysisModuleBadge icon="label"         label="EXT-003" desc="Keywords"/>
                        <AnalysisModuleBadge icon="timeline"      label="EXT-006" desc="Timeline"/>
                        <AnalysisModuleBadge icon="lightbulb"     label="EXT-007" desc="Impacto"/>
                        <AnalysisModuleBadge icon="fact_check"    label="FOR-002" desc="Lenguaje evasivo"/>
                    </div>
                </div>
            })}

            // ── Resultados del análisis ───────────────────────────────────────
            {move || result.get().map(|r| {
                let r2  = r.clone();
                let r3  = r.clone();
                let r4  = r.clone();
                let r5  = r.clone();
                let r6  = r.clone();
                let r7  = r.clone();

                view! {
                    <div class="grid grid-cols-12 gap-6">

                        // ── Card: Legibilidad (FOR-001) ────────────────────────
                        <div class="col-span-12 lg:col-span-4 bg-white p-8 shadow-sm rounded-lg">
                            <div class="flex justify-between items-start mb-6">
                                <div>
                                    <span class="text-[10px] font-black uppercase tracking-widest text-[#C45911]">"FOR-001"</span>
                                    <h3 class="font-sans font-bold text-sm uppercase tracking-widest text-primary mt-1">
                                        "Perfil de Legibilidad"
                                    </h3>
                                </div>
                                <span class="material-symbols-outlined text-primary/30">"menu_book"</span>
                            </div>
                            <div class="font-serif text-sm text-on-surface leading-relaxed whitespace-pre-wrap">
                                {r.readability_raw.clone()}
                            </div>
                        </div>

                        // ── Card: Sentimiento (TON-005 / TON-006) ─────────────
                        <div class="col-span-12 lg:col-span-4 bg-white p-8 shadow-sm rounded-lg">
                            <div class="flex justify-between items-start mb-6">
                                <div>
                                    <span class="text-[10px] font-black uppercase tracking-widest text-[#C45911]">"TON-005"</span>
                                    <h3 class="font-sans font-bold text-sm uppercase tracking-widest text-primary mt-1">
                                        "Tono y Sentimiento"
                                    </h3>
                                </div>
                                <span class="material-symbols-outlined text-primary/30">"psychology"</span>
                            </div>
                            <div class="font-serif text-sm text-on-surface leading-relaxed whitespace-pre-wrap">
                                {r2.sentiment_raw.clone()}
                            </div>
                        </div>

                        // ── Card: Anomalías / Preguntas Inversas (INV-001..005) ─
                        <div class="col-span-12 lg:col-span-4 bg-[#401700] p-8 shadow-sm rounded-lg text-white">
                            <div class="flex justify-between items-start mb-6">
                                <div>
                                    <span class="text-[10px] font-black uppercase tracking-widest text-[#fa813a]">"INV-001..005"</span>
                                    <h3 class="font-sans font-bold text-sm uppercase tracking-widest text-[#ffdbcb] mt-1">
                                        "Anomalías y Alertas"
                                    </h3>
                                </div>
                                <span class="material-symbols-outlined text-[#fa813a]">"report"</span>
                            </div>
                            <div class="font-serif text-sm text-white/90 leading-relaxed whitespace-pre-wrap">
                                {r3.anomalies_raw.clone()}
                            </div>
                        </div>

                        // ── Card: NER — Entidades (EXT-001) ───────────────────
                        <div class="col-span-12 lg:col-span-7 bg-white p-8 shadow-sm rounded-lg">
                            <div class="flex justify-between items-start mb-6">
                                <div>
                                    <span class="text-[10px] font-black uppercase tracking-widest text-[#C45911]">"EXT-001"</span>
                                    <h3 class="font-sans font-bold text-sm uppercase tracking-widest text-primary mt-1">
                                        "Entidades Reconocidas (NER)"
                                    </h3>
                                </div>
                                <button
                                    on:click={
                                        let ner = r4.ner_raw.clone();
                                        move |_| copy_to_clipboard(ner.clone())
                                    }
                                    class="text-[10px] font-bold uppercase text-primary bg-primary/10 px-3 py-1 rounded-sm hover:bg-primary/20 transition-colors"
                                >
                                    "Copiar"
                                </button>
                            </div>
                            <div class="font-serif text-sm text-on-surface leading-relaxed whitespace-pre-wrap max-h-80 overflow-y-auto">
                                {r4.ner_raw.clone()}
                            </div>
                        </div>

                        // ── Card: Metadatos + Keywords (EXT-003..005) ──────────
                        <div class="col-span-12 lg:col-span-5 bg-white p-8 shadow-sm rounded-lg border-l-4 border-primary">
                            <div class="mb-6">
                                <span class="text-[10px] font-black uppercase tracking-widest text-[#C45911]">"EXT-003..005"</span>
                                <h3 class="font-sans font-bold text-sm uppercase tracking-widest text-primary mt-1">
                                    "Palabras Clave y Metadatos"
                                </h3>
                            </div>
                            <div class="font-serif text-sm text-on-surface leading-relaxed whitespace-pre-wrap max-h-80 overflow-y-auto">
                                {r5.keywords_raw.clone()}
                            </div>
                        </div>

                        // ── Card: Línea Temporal (EXT-006) ────────────────────
                        <div class="col-span-12 bg-white p-8 shadow-sm rounded-lg">
                            <div class="flex justify-between items-start mb-6">
                                <div>
                                    <span class="text-[10px] font-black uppercase tracking-widest text-[#C45911]">"EXT-006"</span>
                                    <h3 class="font-sans font-bold text-sm uppercase tracking-widest text-primary mt-1">
                                        "Línea Temporal de Eventos"
                                    </h3>
                                </div>
                                <span class="material-symbols-outlined text-primary/30">"timeline"</span>
                            </div>
                            <div class="font-serif text-sm text-on-surface leading-relaxed whitespace-pre-wrap">
                                {r6.timeline_raw.clone()}
                            </div>
                        </div>

                        // ── Card: Análisis de Impacto (EXT-007) ──────────────
                        <div class="col-span-12 bg-[#001b30] p-8 shadow-sm rounded-lg">
                            <div class="flex justify-between items-start mb-6">
                                <div>
                                    <span class="text-[10px] font-black uppercase tracking-widest text-[#fa813a]">"EXT-007"</span>
                                    <h3 class="font-sans font-bold text-sm uppercase tracking-widest text-[#66a6ea] mt-1">
                                        "Análisis de Impacto"
                                    </h3>
                                </div>
                                <span class="material-symbols-outlined text-[#66a6ea]/30">"lightbulb"</span>
                            </div>
                            <div class="font-serif text-sm text-white/90 leading-relaxed whitespace-pre-wrap">
                                {r7.impact_raw.clone()}
                            </div>
                        </div>

                    </div>

                    // ── Footer ────────────────────────────────────────────────
                    <footer class="mt-12 flex justify-between items-center text-outline-var">
                        <p class="text-[10px] font-bold uppercase tracking-widest">
                            "© 2026 OLIV4600 SOVEREIGN SYSTEMS"
                        </p>
                        <p class="text-[10px] font-bold uppercase tracking-widest">
                            {format!("Documento: {} palabras — análisis completo",
                                ctx.word_count.get_untracked())}
                        </p>
                    </footer>
                }.into_any()
            })}
        </div>
    }
}

// ── Sub-componente: badge de módulo en el empty state ─────────────────────────
#[component]
fn AnalysisModuleBadge(icon: &'static str, label: &'static str, desc: &'static str) -> impl IntoView {
    view! {
        <div class="flex items-start gap-3 p-3 bg-white rounded-sm shadow-sm">
            <span class="material-symbols-outlined text-[#C45911] text-lg shrink-0">{icon}</span>
            <div>
                <span class="block text-[10px] font-black uppercase tracking-widest text-[#C45911]">{label}</span>
                <span class="block text-xs text-primary font-medium mt-0.5">{desc}</span>
            </div>
        </div>
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// VIEW: CHAT (Document Chat)
// ═══════════════════════════════════════════════════════════════════════════════
// Free-form conversational interface over the loaded document.
// Split 50/50: source document on the left, chat thread on the right.
//
// Corresponds to SRS §5 (Chat over Document — CHA-001..CHA-004):
//   CHA-001: Free-form chat with full document context
//   CHA-002: Quick-question chips ("Explain like I'm 12", "What are the risks?")
//   CHA-003: Briefing mode ("Give me a 2-minute oral summary")
//   CHA-004: Session conversation history
//
// TODO: Chat messages are sent via POST /api/chat (the existing SSE stream endpoint).
// The request body includes: { doc_id, conversation_history[], user_message }
// The server prepends the full document text as system context (chunked if needed).
// Responses stream back token-by-token via SSE and are appended to the chat list.
//
// TODO (TRA-001): each chat exchange is logged to SQLite with timestamp, user query,
// model response, and context window usage. Visible in the Audit view.
//
// Highlighted passages in the source document (`.highlight-ref` class) correspond
// to text fragments cited in the AI's last response. This cross-panel linking
// requires the backend to return source offsets alongside the answer.

#[component]
fn ChatView() -> impl IntoView {
    let ctx = use_context::<DocumentCtx>().expect("DocumentCtx");

    // Estado reactivo de la conversación (CHA-004)
    // Cada tupla: ("user" | "assistant", texto)
    let messages: RwSignal<Vec<(String, String)>> = RwSignal::new(Vec::new());
    let (input_text, set_input_text) = signal(String::new());

    // Macro local para no repetir la lógica de envío en cada handler.
    // Como todos los captures son Copy (RwSignal, ReadSignal, WriteSignal),
    // podemos crear closures separadas sin necesitar Clone.
    // Se usa como bloque inline en cada manejador.

    view! {
        <div class="h-full flex overflow-hidden">

            // ── Panel izquierdo: documento fuente ──────────────────────────────
            <section class="w-1/2 bg-surf-low border-r border-slate-200 flex flex-col overflow-hidden">
                // Cabecera
                <div class="h-12 border-b border-slate-100 flex items-center px-6 shrink-0 bg-surf-low/80">
                    <span class="text-[10px] font-bold uppercase tracking-widest text-outline">
                        {move || {
                            let f = ctx.filename.get();
                            if f.is_empty() { "Sin documento cargado".to_string() }
                            else { format!("Fuente: {f}") }
                        }}
                    </span>
                </div>
                // Texto del documento
                <div class="flex-1 overflow-y-auto p-12">
                    {move || {
                        let text = ctx.text.get();
                        if text.is_empty() {
                            view! {
                                <div class="flex flex-col items-center justify-center h-full text-center opacity-40">
                                    <span class="material-symbols-outlined text-[48px] text-primary mb-4">"description"</span>
                                    <p class="font-serif italic text-xl text-primary">
                                        "Carga un documento para chatear con él"
                                    </p>
                                </div>
                            }.into_any()
                        } else {
                            view! {
                                <div class="max-w-2xl mx-auto bg-white shadow-sm p-10">
                                    <div class="mb-8 border-b border-slate-100 pb-6">
                                        <span class="text-[10px] font-bold text-slate-400 tracking-widest uppercase">
                                            {move || ctx.filename.get()}
                                        </span>
                                        <div class="mt-2 flex gap-3 text-[10px] font-bold uppercase text-outline">
                                            <span>{move || format!("{} palabras", ctx.word_count.get())}</span>
                                        </div>
                                    </div>
                                    <article class="font-serif text-base leading-relaxed text-slate-800 whitespace-pre-wrap">
                                        {text}
                                    </article>
                                </div>
                            }.into_any()
                        }
                    }}
                </div>
                // Quick chips (CHA-002) — cada chip tiene su propia closure independiente
                // (todos los signals son Copy, no se necesita Clone)
                <div class="border-t border-slate-200 flex gap-2 flex-wrap p-4 bg-white/80 shrink-0">
                    {[
                        ("child_care",  "Explícamelo como si tuviera 12 años"),
                        ("warning",     "¿Cuáles son los riesgos?"),
                        ("search",      "¿Qué información falta?"),
                        ("mic",         "Resumen de 2 minutos para comunicar en voz alta"),
                    ].map(|(icon, label)| {
                        let label_s = label.to_string();
                        view! {
                            <button
                                class="bg-white/90 backdrop-blur shadow border border-slate-200 px-3 py-1.5 text-[10px] font-bold uppercase tracking-tight text-primary hover:bg-primary hover:text-white transition-all rounded-full flex items-center gap-1.5"
                                on:click=move |_| {
                                    if ctx.processing.get_untracked() { return; }
                                    run_chat(ctx, messages, label_s.clone());
                                }
                            >
                                <span class="material-symbols-outlined text-[13px]">{icon}</span>
                                {label}
                            </button>
                        }
                    }).collect_view()}
                </div>
            </section>

            // ── Panel derecho: interfaz de chat ────────────────────────────────
            <section class="w-1/2 flex flex-col bg-white">

                // Ribbon de estado
                <div class="h-12 border-b border-slate-100 flex items-center justify-between px-6 bg-surf-low/50 shrink-0">
                    <div class="flex items-center gap-4">
                        <div class="flex items-center gap-2">
                            <div class="w-2 h-2 rounded-full bg-emerald-500"></div>
                            <span class="text-[10px] font-black uppercase tracking-widest text-slate-500">
                                "Local Instance Online"
                            </span>
                        </div>
                    </div>
                    <div class="flex gap-3">
                        <button
                            class="text-slate-400 text-sm hover:text-primary"
                            title="Limpiar conversación"
                            on:click=move |_| messages.set(Vec::new())
                        >
                            <span class="material-symbols-outlined text-sm">"delete_sweep"</span>
                        </button>
                    </div>
                </div>

                // Hilo de mensajes
                <div class="flex-1 overflow-y-auto p-8 space-y-6">
                    // Saludo inicial si no hay mensajes
                    {move || messages.get().is_empty().then(|| view! {
                        <div class="flex flex-col items-start max-w-[85%]">
                            <div class="flex items-center gap-2 mb-2">
                                <span class="w-6 h-6 bg-primary text-white flex items-center justify-center text-[10px] font-bold rounded">"O"</span>
                                <span class="text-[11px] font-bold uppercase text-slate-400 tracking-tight">"OLIV4600 Engine"</span>
                            </div>
                            <div class="bg-surf-low p-5 rounded-xl rounded-tl-none border border-slate-100">
                                <p class="font-serif text-base text-slate-700 leading-snug">
                                    {move || {
                                        let f = ctx.filename.get();
                                        if f.is_empty() {
                                            "Hola. Carga un documento para empezar a trabajar con él.".to_string()
                                        } else {
                                            format!("He cargado «{f}». ¿Qué quieres saber sobre este documento?")
                                        }
                                    }}
                                </p>
                            </div>
                        </div>
                    })}

                    // Mensajes reactivos
                    {move || messages.get().into_iter().enumerate().map(|(i, (role, content))| {
                        if role == "user" {
                            view! {
                                <div class="flex flex-col items-end w-full">
                                    <div class="max-w-[75%] bg-primary text-white p-4 rounded-xl rounded-tr-none shadow-md">
                                        <p class="text-sm leading-relaxed">{content}</p>
                                    </div>
                                </div>
                            }.into_any()
                        } else {
                            // Asistente — puede estar en streaming (último mensaje + processing)
                            let is_streaming = move || {
                                ctx.processing.get()
                                    && i == messages.get().len().saturating_sub(1)
                            };
                            view! {
                                <div class="flex flex-col items-start max-w-[88%]">
                                    <div class="flex items-center gap-2 mb-1">
                                        <span class="w-6 h-6 bg-primary text-white flex items-center justify-center text-[10px] font-bold rounded">"O"</span>
                                        <span class="text-[11px] font-bold uppercase text-slate-400 tracking-tight">"OLIV4600 Engine"</span>
                                    </div>
                                    <div class="bg-surf-low p-5 rounded-xl rounded-tl-none border border-slate-100 group">
                                        <p class="font-serif text-base text-slate-700 leading-snug whitespace-pre-wrap">
                                            {content}
                                            // Cursor parpadeante mientras llegan tokens
                                            {move || is_streaming().then(|| view! {
                                                <span class="inline-block w-2 h-4 bg-primary animate-pulse align-middle ml-0.5"></span>
                                            })}
                                        </p>
                                        <button
                                            class="mt-3 flex items-center gap-1 text-[10px] font-bold uppercase tracking-widest text-primary opacity-0 group-hover:opacity-100 transition-opacity"
                                            on:click={
                                                // .get(i) → Option<&(String,String)>; usamos .1 para el texto
                                                let c = messages.get_untracked()
                                                    .get(i)
                                                    .map(|pair| pair.1.clone())
                                                    .unwrap_or_default();
                                                move |_| copy_to_clipboard(c.clone())
                                            }
                                        >
                                            <span class="material-symbols-outlined text-xs">"content_copy"</span>
                                            "Copiar"
                                        </button>
                                    </div>
                                </div>
                            }.into_any()
                        }
                    }).collect_view()}
                </div>

                // Área de entrada de texto
                <div class="p-6 bg-white border-t border-slate-100 shrink-0">
                    <div class="relative flex items-end gap-3 bg-surf-low rounded-xl p-3 border border-slate-200 focus-within:border-primary transition-all">
                        <textarea
                            class="flex-1 bg-transparent border-none focus:ring-0 text-sm text-on-surf placeholder:text-slate-400 resize-none max-h-32"
                            placeholder="Pregunta al motor soberano sobre este documento..."
                            rows="2"
                            prop:value={move || input_text.get()}
                            on:input=move |ev| set_input_text.set(event_target_value(&ev))
                            on:keydown=move |ev| {
                                // Enter sin Shift = enviar
                                if ev.key() == "Enter" && !ev.shift_key() {
                                    ev.prevent_default();
                                    let msg = input_text.get_untracked().trim().to_string();
                                    if !msg.is_empty() && !ctx.processing.get_untracked() {
                                        set_input_text.set(String::new());
                                        run_chat(ctx, messages, msg);
                                    }
                                }
                            }
                        />
                        <button
                            class="bg-[#C45911] text-white p-3 rounded-lg shadow-md hover:bg-[#401700] transition-all active:scale-95 disabled:opacity-40 disabled:cursor-not-allowed"
                            disabled=move || ctx.processing.get() || input_text.get().trim().is_empty()
                            on:click=move |_| {
                                let msg = input_text.get_untracked().trim().to_string();
                                if !msg.is_empty() && !ctx.processing.get_untracked() {
                                    set_input_text.set(String::new());
                                    run_chat(ctx, messages, msg);
                                }
                            }
                        >
                            <span class="material-symbols-outlined">"send"</span>
                        </button>
                    </div>
                    <div class="mt-3 flex justify-between items-center px-1">
                        <div class="flex items-center gap-2">
                            <span class="material-symbols-outlined text-[14px] text-emerald-500">"verified_user"</span>
                            <span class="text-[9px] font-bold uppercase tracking-widest text-slate-400">
                                "Modo Auditoría Segura Activo"
                            </span>
                        </div>
                        <span class="text-[9px] font-bold uppercase tracking-widest text-slate-400">
                            {move || format!("{} mensajes", messages.get().len())}
                        </span>
                    </div>
                </div>
            </section>
        </div>
    }
}

// Chat sub-components

#[component]
fn QuickChip(icon: &'static str, label: &'static str) -> impl IntoView {
    view! {
        <button class="bg-white/90 backdrop-blur shadow-xl border border-slate-200 px-4 py-2 text-[11px] font-bold uppercase tracking-tight text-primary hover:bg-primary hover:text-white transition-all rounded-full flex items-center gap-2">
            <span class="material-symbols-outlined text-[14px]">{icon}</span>
            {label}
        </button>
    }
}

#[component]
fn ChatBubbleAi(text: &'static str) -> impl IntoView {
    view! {
        <div class="flex flex-col items-start max-w-[85%]">
            <div class="flex items-center gap-2 mb-2">
                <span class="w-6 h-6 bg-primary text-white flex items-center justify-center text-[10px] font-bold rounded">
                    "O"
                </span>
                <span class="text-[11px] font-bold uppercase text-slate-400 tracking-tight">
                    "OLIV4600 Engine"
                </span>
            </div>
            <div class="bg-surf-low p-6 rounded-xl rounded-tl-none border border-slate-100 group">
                <p class="font-serif text-lg text-slate-700 leading-snug">{text}</p>
                <button class="mt-4 flex items-center gap-2 text-[10px] font-bold uppercase tracking-widest text-primary opacity-0 group-hover:opacity-100 transition-opacity">
                    <span class="material-symbols-outlined text-xs">"export_notes"</span>
                    "Export Answer"
                </button>
            </div>
        </div>
    }
}

#[component]
fn ChatBubbleUser(text: &'static str, time: &'static str) -> impl IntoView {
    view! {
        <div class="flex flex-col items-end w-full">
            <div class="max-w-[75%] bg-primary text-white p-5 rounded-xl rounded-tr-none shadow-lg">
                <p class="text-sm leading-relaxed">{text}</p>
            </div>
            <span class="text-[10px] font-bold uppercase text-slate-300 mt-2 mr-1">
                {format!("Sent {time}")}
            </span>
        </div>
    }
}

#[component]
fn ChatBubbleAiDetailed() -> impl IntoView {
    view! {
        <div class="flex flex-col items-start max-w-[90%]">
            <div class="flex items-center gap-2 mb-2">
                <span class="w-6 h-6 bg-primary text-white flex items-center justify-center text-[10px] font-bold rounded">
                    "O"
                </span>
                <span class="text-[11px] font-bold uppercase text-slate-400 tracking-tight">
                    "OLIV4600 Engine"
                </span>
            </div>
            <div class="bg-surf-low p-6 rounded-xl rounded-tl-none border border-slate-100 group space-y-4">
                <p class="font-serif text-lg text-slate-700 leading-snug">
                    "Based on the highlighted sections of the report, the security benefits
                    of the local OLIV4600 engine are three-fold:"
                </p>
                <ul class="space-y-3 text-sm text-slate-600">
                    // TODO (VER-002): each numbered point should carry a source citation
                    // e.g. "— §2, para. 3" that links back to the document panel
                    <ChatAnswerPoint n="01" title="Data Sovereignty"
                        text="By operating within a hardened perimeter, no raw document data or inference
                              metadata ever traverses the public internet, mitigating the risk of MITM attacks."
                    />
                    <ChatAnswerPoint n="02" title="Latency Elimination"
                        text="Local execution ensures deterministic response times for kinetic-response
                              applications where millisecond delays are critical."
                    />
                    <ChatAnswerPoint n="03" title="Verification"
                        text="The engine supports hardware-level auditing, allowing human operators to verify
                              the exact compute paths used to generate an intelligence output."
                    />
                </ul>
                <div class="pt-4 flex gap-4">
                    <button class="flex items-center gap-2 text-[10px] font-bold uppercase tracking-widest text-primary">
                        <span class="material-symbols-outlined text-xs">"content_copy"</span>
                        "Copy"
                    </button>
                    <button class="flex items-center gap-2 text-[10px] font-bold uppercase tracking-widest text-primary">
                        <span class="material-symbols-outlined text-xs">"export_notes"</span>
                        "Export Answer"
                    </button>
                </div>
            </div>
        </div>
    }
}

#[component]
fn ChatAnswerPoint(n: &'static str, title: &'static str, text: &'static str) -> impl IntoView {
    view! {
        <li class="flex gap-3">
            <span class="font-bold text-primary shrink-0">{format!("{n}.")}</span>
            <span>
                <strong class="text-primary">{title}": "</strong>
                {text}
            </span>
        </li>
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// VIEW: PIPELINE (Production Pipeline / Verifiability)
// ═══════════════════════════════════════════════════════════════════════════════
// Visualizes the document derivation chain: from the root source document
// through all generated outputs (executive summary → press release → social posts).
// This is the "genealogy" view that proves every derived document's provenance.
//
// Corresponds to SRS modules:
//   11 (Document Production Chain — CAD-001..CAD-005)
//   17 (Expedition Package — EXP-001..EXP-005)
//   13 (Verifiability — VER-002, VER-005 traceability map export)
//   4.4 (Traceability — TRA-002 document genealogy)
//
// TODO (CAD-002): the pipeline should be configurable — users drag nodes to
// define the derivation chain: acta → convocatoria → nota de prensa → posts → FAQs.
// Each node is a transformation type from Module 3 or Module 4.
//
// TODO (CAD-003): "Propagate changes" — when the source document is updated,
// automatically flag all derived documents as "stale" (orange badge) and offer
// a one-click "Regenerate All" button to refresh the entire chain.
//
// TODO (EXP-005): "Export Package" → ZIP with all derived documents in a structured
// folder hierarchy: /expedition-{id}/source.docx, /summary.pdf, /press-release.docx, etc.

#[component]
fn PipelineView() -> impl IntoView {
    let show_note = RwSignal::new(true);
    view! {
        <section class="p-12 flex flex-col min-h-full">

            // ── Header ─────────────────────────────────────────────────────────
            <div class="flex justify-between items-end mb-16">
                <div>
                    <h2 class="font-sans text-4xl font-black text-primary tracking-tight mb-2">
                        "Production Pipeline"
                    </h2>
                    <p class="font-serif text-slate-500 text-xl max-w-2xl">
                        "Visualizing the propagation of institutional intelligence from source to distribution nodes."
                    </p>
                </div>
                // TODO (CAD-002): "Regenerate Full Pipeline" → POST /api/pipeline/regenerate
                // with { project_id }. Server queues all pending transformation jobs and
                // streams progress events back. Rotate the icon while processing.
                <button class="group flex items-center gap-3 bg-[#401700] text-white px-8 py-4 rounded-lg font-sans font-bold text-sm tracking-widest uppercase hover:bg-[#622700] transition-all shadow-xl">
                    <span class="material-symbols-outlined group-hover:rotate-180 transition-transform duration-500">
                        "cached"
                    </span>
                    "Regenerate Full Pipeline"
                </button>
            </div>

            // ── Horizontal Pipeline Visualizer ─────────────────────────────────
            // Nodes are connected by horizontal lines (CSS ::after pseudo-element).
            // Each node represents either the root document or a derived output.
            // TODO: make this list dynamic — fetch nodes from GET /api/pipeline/{project_id}
            // which returns the derivation DAG from SQLite (TRA-002).
            // TODO (CAD-005): add a temporal sequencing view that shows:
            //   convocatoria → recordatorio → nota del día → resumen posterior
            // with scheduled send dates attached to each node.
            <div class="flex-1 flex items-center overflow-x-auto pb-12">
                <div class="flex gap-12 items-stretch">

                    // Node: Root Source Document
                    // The "ROOT AUTHORITY" — all derived documents trace back here.
                    // The "SOURCE UPDATED" badge appears when the source has been modified
                    // after derived documents were generated (CAD-003 propagation pending).
                    <PipelineSourceNode/>

                    // Node: Executive Summary (Derived Layer 01)
                    // Status: Edited — human has modified the AI output.
                    // TODO: "Edited" badge appears when the output has been manually changed
                    // after generation. Store a dirty flag in SQLite alongside the output.
                    <PipelineDerivedNode
                        layer="Derived Layer 01"
                        icon="summarize"
                        icon_color="text-blue-500"
                        accent_color="bg-blue-500"
                        title="Executive Summary"
                        desc="Synthesized high-level overview for leadership alignment."
                        status="Edited"
                        status_class="bg-blue-50 text-blue-600"
                        locked=false
                        waiting_for=""
                    />

                    // Node: Press Release (Derived Layer 02)
                    // Status: Generated — AI output not yet reviewed by human.
                    // TODO: "Review Now" CTA should navigate to EditorView with this
                    // specific derived document loaded in the result panel for editing.
                    <PipelineDerivedNode
                        layer="Derived Layer 02"
                        icon="news"
                        icon_color="text-emerald-500"
                        accent_color="bg-emerald-500"
                        title="Press Release"
                        desc="External-facing communiqué for global media distribution."
                        status="Generated"
                        status_class="bg-emerald-50 text-emerald-600"
                        locked=false
                        waiting_for=""
                    />

                    // Node: LinkedIn Post (Social Amplification)
                    // Status: Pending — waiting for Press Release (Layer 02) to be approved.
                    // TODO (CAD-002): enforce dependency order — don't generate social posts
                    // until the upstream press release has been reviewed (status = "Approved").
                    // This prevents propagating errors from the press release into social copy.
                    <PipelineDerivedNode
                        layer="Social Amplification"
                        icon="share"
                        icon_color="text-slate-400"
                        accent_color="bg-slate-300"
                        title="LinkedIn Post"
                        desc="Narrative-driven professional update for corporate ecosystem."
                        status="Pending"
                        status_class="bg-slate-200 text-slate-500"
                        locked=true
                        waiting_for="Awaiting Layer 02..."
                    />

                    // Node: Twitter/X Thread (Viral Extraction)
                    // Status: Pending — also waiting for Press Release approval.
                    // TODO (GEN-004): Twitter thread generation respects the 280-char limit
                    // per tweet. The LLM prompt includes: "Split this into N tweets,
                    // each ≤280 chars, numbered 1/N...N/N".
                    <PipelineDerivedNode
                        layer="Viral Extraction"
                        icon="rebase_edit"
                        icon_color="text-slate-400"
                        accent_color="bg-slate-300"
                        title="Twitter / X Thread"
                        desc="Concise, high-impact data points for rapid discourse."
                        status="Pending"
                        status_class="bg-slate-200 text-slate-500"
                        locked=true
                        waiting_for="Awaiting Layer 02..."
                    />

                    // TODO (GEN-007 + GEN-009): add more nodes as needed:
                    //   - Email Newsletter (GEN-007)
                    //   - FAQ Document (GEN-009)
                    //   - Blog Article (GEN-005)
                    // Nodes beyond the viewport scroll horizontally.
                </div>
            </div>

            // ── Footer Metadata ────────────────────────────────────────────────
            // Pipeline health and synchronization status.
            // TODO: drive "Last Synced" from the project's last-modified timestamp.
            // "Pipeline Health" should count active/failed/pending jobs.
            // "AI Confidence" is the average VER-004 confidence score across all generated nodes.
            <div class="grid grid-cols-4 gap-8 pt-8 border-t border-slate-200">
                <div class="flex flex-col gap-1">
                    <span class="font-sans text-[10px] font-black uppercase tracking-widest text-slate-400">
                        "Last Synced"
                    </span>
                    <span class="font-serif italic text-primary">"12 Oct 2026, 14:32:01 GMT"</span>
                </div>
                <div class="flex flex-col gap-1">
                    <span class="font-sans text-[10px] font-black uppercase tracking-widest text-slate-400">
                        "Pipeline Health"
                    </span>
                    <div class="flex items-center gap-2">
                        <div class="w-2 h-2 rounded-full bg-emerald-500"></div>
                        <span class="font-serif italic text-primary">"Stable — 2 active workers"</span>
                    </div>
                </div>
                <div class="flex flex-col gap-1">
                    <span class="font-sans text-[10px] font-black uppercase tracking-widest text-slate-400">
                        "AI Confidence"
                    </span>
                    <div class="w-full bg-slate-200 h-1 mt-2">
                        <div class="bg-primary h-1 w-[94%]"></div>
                    </div>
                </div>
                <div class="flex flex-col gap-1 text-right">
                    <span class="font-sans text-[10px] font-black uppercase tracking-widest text-slate-400">
                        "System State"
                    </span>
                    <span class="font-sans font-bold text-emerald-600 uppercase text-[10px]">
                        "Ready for Propagation"
                    </span>
                </div>
            </div>

            // ── Floating Architect's Note (Glassmorphism panel) ────────────────
            // Architect's Note — dismissable con señal reactiva
            {move || show_note.get().then(|| view! {
                <div class="fixed bottom-12 right-12 w-80 bg-white/70 backdrop-blur-2xl p-6 shadow-2xl border border-white/40 rounded-lg z-50">
                    <div class="flex items-start gap-4">
                        <div class="p-2 bg-[#C45911]/10 text-[#C45911] rounded">
                            <span class="material-symbols-outlined text-lg">"insights"</span>
                        </div>
                        <div class="flex-1">
                            <h5 class="font-sans font-bold text-xs uppercase tracking-tight text-primary mb-2">
                                "Architect's Note"
                            </h5>
                            <p class="text-sm font-serif text-slate-600 leading-relaxed">
                                "Source document changes detected in "
                                <span class="font-bold text-primary">"Section 4.2"</span>
                                ". Recommended regeneration of the "
                                <span class="italic">"Press Release"</span>
                                " node to maintain integrity."
                            </p>
                            <div class="mt-4 flex gap-3">
                                <button class="text-[10px] font-sans font-black uppercase tracking-wider text-primary border-b border-primary pb-0.5">
                                    "Accept"
                                </button>
                                <button
                                    class="text-[10px] font-sans font-black uppercase tracking-wider text-slate-400 hover:text-red-500 transition-colors"
                                    on:click=move |_| show_note.set(false)
                                >
                                    "Dismiss"
                                </button>
                            </div>
                        </div>
                    </div>
                </div>
            })}
        </section>
    }
}

// Pipeline sub-components

#[component]
fn PipelineSourceNode() -> impl IntoView {
    view! {
        <div class="flex flex-col gap-4">
            <span class="font-sans text-[10px] font-black uppercase tracking-[0.2em] text-slate-400">
                "Root Authority"
            </span>
            <div class="w-72 bg-white p-1 shadow-lg border border-slate-200">
                <div class="bg-surf-cont p-6 border-b border-slate-100 flex flex-col gap-4">
                    <div class="flex justify-between items-start">
                        <div class="bg-primary text-white p-2">
                            <span class="material-symbols-outlined text-lg">"description"</span>
                        </div>
                        // This badge indicates the source has been modified after derivations
                        // TODO (CAD-003): show/hide based on dirty flag from backend
                        <span class="px-2 py-0.5 bg-orange-100 text-[#fa813a] text-[10px] font-bold flex items-center gap-1 border border-[#fa813a]/20">
                            <span class="material-symbols-outlined text-[10px]">"error"</span>
                            "SOURCE UPDATED"
                        </span>
                    </div>
                    <div>
                        <h3 class="font-serif font-bold text-lg text-primary leading-tight">
                            "Institutional Report: Q3 Sovereignty Flux"
                        </h3>
                        <p class="text-[11px] text-slate-400 uppercase mt-1 tracking-wider">
                            "DOC_ID: OLIV-8820-X"
                        </p>
                    </div>
                </div>
                // Document preview (text skeleton + page count)
                <div class="relative h-48 bg-white p-4 overflow-hidden">
                    <div class="space-y-2 opacity-40">
                        <div class="h-2 w-full bg-slate-200"></div>
                        <div class="h-2 w-4/5 bg-slate-200"></div>
                        <div class="h-2 w-full bg-slate-200"></div>
                        <div class="h-2 w-2/3 bg-slate-200"></div>
                        <div class="mt-4 h-20 w-full bg-slate-100"></div>
                    </div>
                    <div class="absolute inset-0 bg-gradient-to-t from-white via-transparent to-transparent"></div>
                    <div class="absolute bottom-4 left-4 right-4 flex justify-between items-center">
                        <span class="text-[10px] font-bold text-slate-400">"142 PAGES"</span>
                        <span class="material-symbols-outlined text-primary text-sm">"open_in_new"</span>
                    </div>
                </div>
            </div>
            <div class="flex items-center gap-2 text-[#C45911] px-1">
                <span class="material-symbols-outlined text-sm">"warning"</span>
                <span class="text-[11px] font-sans font-bold uppercase tracking-tighter">
                    "Propagation Pending"
                </span>
            </div>
        </div>
    }
}

#[component]
fn PipelineDerivedNode(
    layer:        &'static str,
    icon:         &'static str,
    icon_color:   &'static str,
    accent_color: &'static str,
    title:        &'static str,
    desc:         &'static str,
    status:       &'static str,
    status_class: &'static str,
    locked:       bool,
    waiting_for:  &'static str,
) -> impl IntoView {
    let card_class = if locked {
        "w-64 bg-slate-50 border border-slate-200 p-6 flex flex-col gap-4 relative overflow-hidden opacity-80"
    } else {
        "w-64 bg-white border border-slate-200 p-6 shadow-md flex flex-col gap-4 relative overflow-hidden"
    };
    view! {
        <div class="flex flex-col gap-4">
            <span class="font-sans text-[10px] font-black uppercase tracking-[0.2em] text-slate-400">
                {layer}
            </span>
            <div class=card_class>
                // Colored accent strip on right edge — indicates derivation layer
                <div class=format!("absolute top-0 right-0 w-1 h-full {}", accent_color)></div>
                <div class="flex justify-between items-center">
                    <span class=format!("material-symbols-outlined {}", icon_color)>{icon}</span>
                    <span class=format!("px-2 py-0.5 {} text-[10px] font-bold uppercase tracking-wider", status_class)>
                        {status}
                    </span>
                </div>
                <h4 class="font-serif font-bold text-base text-primary">{title}</h4>
                <p class="text-sm font-serif text-slate-500 leading-relaxed italic">{desc}</p>
                <div class="mt-auto pt-4 border-t border-slate-100 flex justify-between items-center">
                    {if locked {
                        view! {
                            <span class="text-[10px] text-slate-400 italic">{waiting_for}</span>
                            <span class="material-symbols-outlined text-slate-300">"lock"</span>
                        }.into_any()
                    } else {
                        view! {
                            // TODO: "Review Now" → navigate to EditorView with this derived
                            // document pre-loaded in the result panel for human editing/approval
                            <button class="text-[10px] font-bold text-blue-600 hover:underline">
                                "Review Now"
                            </button>
                            <span class="material-symbols-outlined text-slate-300 hover:text-primary cursor-pointer">
                                "more_horiz"
                            </span>
                        }.into_any()
                    }}
                </div>
            </div>
        </div>
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// VIEW: AUDIT (Audit Log)
// ═══════════════════════════════════════════════════════════════════════════════
// Tamper-evident operation log for all document processing activities.
// Required for ENS (Esquema Nacional de Seguridad) Category Alta compliance.
//
// Corresponds to SRS §4.4 (Traceability & Audit):
//   TRA-001: log of each transformation (input, prompt, output, user, timestamp)
//   TRA-002: document genealogy (which document derives from which)
//   TRA-003: export audit logs in standard format (JSON-LD or CSV)
//   TRA-004: exportable traceability map (supported assertions → source)
//   SEC-005: audit log of all operations
//
// TODO: fetch log entries from GET /api/audit?project_id=... ordered by timestamp DESC.
// Each entry should show:
//   - Timestamp (ISO 8601)
//   - Operation type (TRANSFORM / ANALYZE / CHAT / EXPORT / PURGE)
//   - Document ID and filename
//   - Transformation module used (e.g. "Module 2 — Executive Summary")
//   - Input hash (SHA-256 of source text)
//   - Output hash (SHA-256 of result text)
//   - Prompt version used
//   - User/session ID (if role-based access is implemented — SEC-003)
//   - Duration (ms)
//
// TODO (SEC-002): audit entries themselves should be hash-chained (each entry
// includes the hash of the previous entry) to detect tampering — similar to a
// blockchain ledger but stored in SQLite.
//
// TODO (TRA-003): "Export Audit Log" button → GET /api/audit/export?format=json|csv
// Returns the full log as a downloadable file.
//
// TODO (PUB-006): run GDPR/LOPDGDD compliance check on export — flag any entries
// that involve personal data (detected via Module 14 — PUB-001) and prompt the
// user to apply redactions before downloading the log.

// ═══════════════════════════════════════════════════════════════════════════════
// VIEW: ARCHIVE (Biblioteca de proyectos)
// ═══════════════════════════════════════════════════════════════════════════════
// Lista completa de proyectos procesados, con búsqueda, filtros y acceso rápido
// al análisis cacheado y las transformaciones de cada documento.
//
// Fuente de datos: GET /api/projects?limit=N → tabla `projects` en SQLite.
// Cada proyecto se crea automáticamente en /api/extract y se enriquece con
// has_analysis=true cuando se completa un análisis (POST /api/analysis).

#[component]
fn ArchiveView(set_active_view: WriteSignal<View>) -> impl IntoView {
    let ctx = use_context::<DocumentCtx>().expect("DocumentCtx");

    // ── Estado reactivo ───────────────────────────────────────────────────────
    let projects:   RwSignal<Option<Vec<ApiProject>>> = RwSignal::new(None);
    let search:     RwSignal<String>                  = RwSignal::new(String::new());
    let loading:    RwSignal<bool>                    = RwSignal::new(true);

    // ── Cargar proyectos al montar ────────────────────────────────────────────
    // El plugin lee su propia tabla oliv_projects vía el endpoint genérico del core.
    spawn_local(async move {
        let rows = plugin_query(
            "SELECT doc_hash, doc_name, original_path, word_count, \
             transform_count, has_analysis, created_at, updated_at \
             FROM oliv_projects ORDER BY updated_at DESC LIMIT 200",
            vec![],
        ).await;
        let data = rows.into_iter().filter_map(|r| {
            Some(ApiProject {
                doc_hash:        r["doc_hash"].as_str()?.to_string(),
                doc_name:        r["doc_name"].as_str()?.to_string(),
                original_path:   r["original_path"].as_str()?.to_string(),
                word_count:      r["word_count"].as_u64()? as u32,
                transform_count: r["transform_count"].as_u64()? as u32,
                has_analysis:    r["has_analysis"].as_i64()? != 0,
                created_at:      r["created_at"].as_str()?.to_string(),
                updated_at:      r["updated_at"].as_str()?.to_string(),
            })
        }).collect::<Vec<_>>();
        projects.set(Some(data));
        loading.set(false);
    });

    // ── Proyectos filtrados por búsqueda ──────────────────────────────────────
    let filtered = move || {
        let q = search.get().to_lowercase();
        projects.get()
            .unwrap_or_default()
            .into_iter()
            .filter(|p| q.is_empty() || p.doc_name.to_lowercase().contains(&q))
            .collect::<Vec<_>>()
    };

    // ── Abrir un proyecto: carga el texto desde el servidor y navega al Editor ─
    let open_project = move |hash: String, _: web_sys::MouseEvent| {
        // Recuperar el proyecto del estado local para obtener original_path
        if let Some(projs) = projects.get_untracked() {
            if let Some(p) = projs.iter().find(|p| p.doc_hash == hash) {
                let path = p.original_path.clone();
                spawn_local(async move {
                    // Re-extraer el texto (el archivo ya está en uploads/)
                    let body = serde_json::json!({ "path": path }).to_string();
                    let headers = web_sys::Headers::new().unwrap();
                    headers.set("Content-Type", "application/json").unwrap();
                    let opts = web_sys::RequestInit::new();
                    opts.set_method("POST");
                    opts.set_body(&wasm_bindgen::JsValue::from_str(&body));
                    opts.set_headers(&wasm_bindgen::JsValue::from(headers));
                    if let Ok(req) = web_sys::Request::new_with_str_and_init("/api/extract", &opts) {
                        if let Some(w) = web_sys::window() {
                            if let Ok(rv) = JsFuture::from(w.fetch_with_request(&req)).await {
                                let resp: web_sys::Response = rv.unchecked_into();
                                if resp.ok() {
                                    if let Ok(jv) = JsFuture::from(resp.json().unwrap()).await {
                                        let get = |k: &str| js_sys::Reflect::get(&jv, &wasm_bindgen::JsValue::from_str(k))
                                            .ok().and_then(|v| v.as_string()).unwrap_or_default();
                                        let text  = get("text");
                                        let fname = get("filename");
                                        let wc    = js_sys::Reflect::get(&jv, &wasm_bindgen::JsValue::from_str("word_count"))
                                            .ok().and_then(|v| v.as_f64()).unwrap_or(0.0) as u32;
                                        ctx.text.set(text);
                                        ctx.filename.set(fname);
                                        ctx.word_count.set(wc);
                                        set_active_view.set(View::Editor);
                                    }
                                }
                            }
                        }
                    }
                });
            }
        }
    };

    // ── Abrir análisis: carga el proyecto y navega a Analysis ─────────────────
    let open_analysis = move |hash: String, _: web_sys::MouseEvent| {
        if let Some(projs) = projects.get_untracked() {
            if let Some(p) = projs.iter().find(|p| p.doc_hash == hash) {
                let path = p.original_path.clone();
                spawn_local(async move {
                    let body = serde_json::json!({ "path": path }).to_string();
                    let headers = web_sys::Headers::new().unwrap();
                    headers.set("Content-Type", "application/json").unwrap();
                    let opts = web_sys::RequestInit::new();
                    opts.set_method("POST");
                    opts.set_body(&wasm_bindgen::JsValue::from_str(&body));
                    opts.set_headers(&wasm_bindgen::JsValue::from(headers));
                    if let Ok(req) = web_sys::Request::new_with_str_and_init("/api/extract", &opts) {
                        if let Some(w) = web_sys::window() {
                            if let Ok(rv) = JsFuture::from(w.fetch_with_request(&req)).await {
                                let resp: web_sys::Response = rv.unchecked_into();
                                if resp.ok() {
                                    if let Ok(jv) = JsFuture::from(resp.json().unwrap()).await {
                                        let get = |k: &str| js_sys::Reflect::get(&jv, &wasm_bindgen::JsValue::from_str(k))
                                            .ok().and_then(|v| v.as_string()).unwrap_or_default();
                                        ctx.text.set(get("text"));
                                        ctx.filename.set(get("filename"));
                                        let wc = js_sys::Reflect::get(&jv, &wasm_bindgen::JsValue::from_str("word_count"))
                                            .ok().and_then(|v| v.as_f64()).unwrap_or(0.0) as u32;
                                        ctx.word_count.set(wc);
                                        set_active_view.set(View::Analysis);
                                    }
                                }
                            }
                        }
                    }
                });
            }
        }
    };

    view! {
        <div class="p-10 max-w-7xl mx-auto">

            // ── Header ────────────────────────────────────────────────────────
            <header class="mb-8">
                <div class="flex justify-between items-end flex-wrap gap-4">
                    <div>
                        <h2 class="text-4xl font-sans font-black tracking-tighter text-primary uppercase">
                            "Biblioteca de Proyectos"
                        </h2>
                        <p class="font-serif italic text-xl text-outline mt-1">
                            {move || {
                                let n = projects.get().map(|v| v.len()).unwrap_or(0);
                                format!("{n} documento{} procesado{}", if n == 1 { "" } else { "s" }, if n == 1 { "" } else { "s" })
                            }}
                        </p>
                    </div>
                    // Buscador
                    <div class="flex items-center gap-2 px-3 py-2 bg-white border border-outline-variant/20 rounded-sm shadow-sm w-72">
                        <span class="material-symbols-outlined text-outline text-[20px]">"search"</span>
                        <input
                            type="text"
                            placeholder="Buscar documento…"
                            class="bg-transparent border-none outline-none text-sm font-label w-full placeholder:text-outline-variant"
                            on:input=move |e| {
                                use wasm_bindgen::JsCast;
                                let val = e.target().unwrap()
                                    .unchecked_into::<web_sys::HtmlInputElement>()
                                    .value();
                                search.set(val);
                            }
                        />
                    </div>
                </div>
            </header>

            // ── Loading ───────────────────────────────────────────────────────
            {move || loading.get().then(|| view! {
                <div class="flex items-center gap-3 py-20 justify-center text-outline">
                    <div class="w-2 h-2 rounded-full bg-outline animate-pulse"></div>
                    <span class="text-sm font-label">"Cargando proyectos…"</span>
                </div>
            })}

            // ── Empty state ───────────────────────────────────────────────────
            {move || (!loading.get() && filtered().is_empty()).then(|| view! {
                <div class="flex flex-col items-center justify-center py-32 text-center">
                    <span class="material-symbols-outlined text-6xl text-outline/20 mb-6">"folder_open"</span>
                    <h3 class="font-sans font-black text-xl text-primary mb-2">
                        {if search.get().is_empty() { "Sin proyectos todavía" } else { "Sin resultados" }}
                    </h3>
                    <p class="font-serif italic text-outline max-w-sm">
                        {if search.get().is_empty() {
                            "Sube un documento desde el Editor para crear tu primer proyecto."
                        } else {
                            "Prueba con otro término de búsqueda."
                        }}
                    </p>
                </div>
            })}

            // ── Tabla de proyectos ────────────────────────────────────────────
            {move || (!loading.get() && !filtered().is_empty()).then(|| {
                let rows = filtered();
                view! {
                    <div class="bg-white rounded-lg shadow-sm overflow-hidden">
                        // Cabecera
                        <div class="grid grid-cols-12 gap-4 px-6 py-3 bg-surface-container-low border-b border-outline-variant/10">
                            <div class="col-span-5 text-[10px] font-black uppercase tracking-widest text-outline">"Documento"</div>
                            <div class="col-span-2 text-[10px] font-black uppercase tracking-widest text-outline">"Palabras"</div>
                            <div class="col-span-2 text-[10px] font-black uppercase tracking-widest text-outline">"Transforms."</div>
                            <div class="col-span-1 text-[10px] font-black uppercase tracking-widest text-outline">"Análisis"</div>
                            <div class="col-span-2 text-[10px] font-black uppercase tracking-widest text-outline text-right">"Acciones"</div>
                        </div>
                        // Filas
                        <div class="divide-y divide-outline-variant/10">
                            {rows.into_iter().map(|p| {
                                let hash1 = p.doc_hash.clone();
                                let hash2 = p.doc_hash.clone();
                                let has_a = p.has_analysis;
                                // Fecha legible: tomar los 10 primeros chars del ISO string
                                let date = if p.updated_at.len() >= 10 {
                                    p.updated_at[..10].to_string()
                                } else { p.updated_at.clone() };
                                // Nombre limpio (solo el filename)
                                let name = p.doc_name.split('/').last()
                                    .unwrap_or(&p.doc_name).to_string();
                                let wc   = p.word_count;
                                let tc   = p.transform_count;
                                view! {
                                    <div class="grid grid-cols-12 gap-4 px-6 py-4 items-center hover:bg-surface-container-lowest/50 transition-colors group">
                                        // Nombre + fecha
                                        <div class="col-span-5">
                                            <div class="flex items-center gap-3">
                                                <span class="material-symbols-outlined text-outline/40 text-[20px] shrink-0">"description"</span>
                                                <div class="min-w-0">
                                                    <p class="font-serif font-bold text-sm text-primary truncate">{name}</p>
                                                    <p class="text-[10px] text-outline font-label">{date}</p>
                                                </div>
                                            </div>
                                        </div>
                                        // Word count
                                        <div class="col-span-2 text-sm font-label text-on-surface-variant">
                                            {format!("{wc}")}
                                        </div>
                                        // Transformaciones
                                        <div class="col-span-2">
                                            <div class="flex items-center gap-2">
                                                <span class="text-sm font-label text-on-surface-variant">{format!("{tc}")}</span>
                                                {(tc > 0).then(|| view! {
                                                    <div class="h-1 flex-1 bg-surface-container-high rounded-full overflow-hidden max-w-16">
                                                        <div
                                                            class="h-full bg-primary rounded-full"
                                                            style=format!("width: {}%", (tc * 10).min(100))
                                                        ></div>
                                                    </div>
                                                })}
                                            </div>
                                        </div>
                                        // Badge análisis
                                        <div class="col-span-1">
                                            {if has_a {
                                                view! {
                                                    <span class="inline-flex items-center gap-1 px-2 py-0.5 bg-[#003b65] text-[#66a6ea] text-[9px] font-black uppercase tracking-widest rounded-sm">
                                                        <span class="w-1 h-1 rounded-full bg-[#66a6ea]"></span>
                                                        "BD"
                                                    </span>
                                                }.into_any()
                                            } else {
                                                view! {
                                                    <span class="text-[10px] text-outline font-label">"—"</span>
                                                }.into_any()
                                            }}
                                        </div>
                                        // Acciones
                                        <div class="col-span-2 flex items-center justify-end gap-2">
                                            // Abrir en editor
                                            <button
                                                on:click={
                                                    let h = hash1.clone();
                                                    let f = open_project;
                                                    move |e| f(h.clone(), e)
                                                }
                                                title="Abrir en Editor"
                                                class="p-1.5 text-outline hover:text-primary transition-colors rounded"
                                            >
                                                <span class="material-symbols-outlined text-[18px]">"edit_note"</span>
                                            </button>
                                            // Abrir análisis (solo si tiene caché)
                                            <button
                                                on:click={
                                                    let h = hash2.clone();
                                                    let f = open_analysis;
                                                    move |e| f(h.clone(), e)
                                                }
                                                title={if has_a { "Ver análisis cacheado" } else { "Ejecutar análisis" }}
                                                class=move || format!(
                                                    "p-1.5 transition-colors rounded {}",
                                                    if has_a { "text-[#66a6ea] hover:text-primary" }
                                                    else { "text-outline hover:text-primary" }
                                                )
                                            >
                                                <span class="material-symbols-outlined text-[18px]">"analytics"</span>
                                            </button>
                                        </div>
                                    </div>
                                }
                            }).collect_view()}
                        </div>
                    </div>
                }.into_any()
            })}

            // ── Footer ────────────────────────────────────────────────────────
            <footer class="mt-12 flex justify-between items-center text-outline-variant">
                <p class="text-[10px] font-bold uppercase tracking-widest">
                    "© 2026 OLIV4600 SOVEREIGN SYSTEMS"
                </p>
                <p class="text-[10px] font-bold uppercase tracking-widest">
                    "~/.local-ai/projects/ · datos 100% locales"
                </p>
            </footer>
        </div>
    }
}

#[component]
fn AuditView() -> impl IntoView {
    view! {
        <div class="p-10 max-w-7xl mx-auto">
            <header class="mb-10">
                <h2 class="text-4xl font-sans font-black tracking-tighter text-primary uppercase">
                    "Audit Log"
                </h2>
                <p class="font-serif italic text-xl text-outline mt-1">
                    "Tamper-evident operation record — ENS Category Alta"
                </p>
            </header>

            // Placeholder — full implementation in Phase 8
            <div class="bg-white rounded-xl p-16 text-center border border-slate-200/50 shadow-sm">
                <span class="material-symbols-outlined text-[48px] text-primary/20 mb-6 block">
                    "history_edu"
                </span>
                <h3 class="font-sans font-black text-xl text-primary mb-2">
                    "Audit Engine — Phase 8"
                </h3>
                <p class="font-serif italic text-on-surf-var max-w-md mx-auto">
                    "The tamper-evident audit log will record every transformation, analysis,
                    and export operation with SHA-256 hash chaining. Scheduled for Phase 8
                    (ENS Category Alta compliance)."
                </p>
                <div class="mt-8 flex justify-center gap-4">
                    <div class="px-4 py-2 bg-surf-low rounded-sm text-[10px] font-bold uppercase tracking-widest text-outline">
                        "TRA-001 ✓ Designed"
                    </div>
                    <div class="px-4 py-2 bg-surf-low rounded-sm text-[10px] font-bold uppercase tracking-widest text-outline">
                        "SEC-005 ✓ Designed"
                    </div>
                    <div class="px-4 py-2 bg-surf-low rounded-sm text-[10px] font-bold uppercase tracking-widest text-outline">
                        "TRA-003 ✓ Designed"
                    </div>
                </div>
            </div>
        </div>
    }
}
