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
    pub text:   String,
    pub label:  String,
    pub format: String,
    pub title:  Option<String>,
    pub date:   Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/export/render", post(render_document))
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

    let dest = workspace.join(safe_filename(&req.label, ext));
    if let Err(e) = tokio::fs::write(&dest, &bytes).await {
        return render_error(&req.format, format!("escribir {ext}: {e}"));
    }

    reveal_in_files(&dest);
    info!("render/{}: guardado → {} ({} bytes)", req.format, dest.display(), bytes.len());

    (StatusCode::OK, axum::Json(serde_json::json!({
        "ok":       true,
        "path":     dest.to_string_lossy(),
        "filename": dest.file_name().and_then(|n| n.to_str()).unwrap_or(""),
    }))).into_response()
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
