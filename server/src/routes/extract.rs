// routes/extract.rs
// POST /api/extract  – extracts plain text from an uploaded file.
// Body: { "path": "/absolute/path/to/file" }
// Returns: { "text": "...", "word_count": N, "filename": "..." }
//
// Supports: .txt .md .html .csv .json .yaml (read directly)
//           .docx / .odt (ZIP+XML extraction)
//           .pdf          (pdf_extract crate)

use axum::{Router, extract::State, routing::post, response::IntoResponse, http::StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};

use crate::AppState;

#[derive(Deserialize)]
pub struct ExtractRequest {
    pub path: String,
}

#[derive(Serialize)]
pub struct ExtractResponse {
    pub text:       String,
    pub word_count: usize,
    pub filename:   String,
    pub doc_hash:   String,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/extract", post(extract_text))
}

async fn extract_text(
    State(state): State<AppState>,
    axum::Json(req): axum::Json<ExtractRequest>,
) -> impl IntoResponse {
    let path = crate::tools::files::resolve_path(&req.path, &state.config);

    if !path.exists() {
        return (StatusCode::NOT_FOUND, "File not found").into_response();
    }

    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let text = match ext.as_str() {
        "pdf" => {
            let args = serde_json::json!({ "path": path.to_string_lossy() });
            match crate::tools::files::read_file(&args, &state.config).await {
                Ok(t) => t,
                Err(e) => return (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()).into_response(),
            }
        }
        "docx" | "odt" => {
            let args = serde_json::json!({ "path": path.to_string_lossy() });
            match crate::tools::files::read_docx(&args, &state.config).await {
                Ok(t) => t,
                Err(e) => return (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()).into_response(),
            }
        }
        _ => {
            match tokio::fs::read_to_string(&path).await {
                Ok(t) => t,
                Err(e) => return (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()).into_response(),
            }
        }
    };

    let word_count = text.split_whitespace().count();
    let doc_hash = hex::encode(Sha256::digest(text.as_bytes()));

    // Semilla de carpeta de proyecto: copia el original y escribe meta.json
    // si el proyecto aún no existe. Best-effort — fallos se loguean y no
    // interrumpen la respuesta.
    seed_project_folder(&state.config, &doc_hash, &path, &filename, &ext, word_count).await;

    axum::Json(ExtractResponse { text, word_count, filename, doc_hash }).into_response()
}

async fn seed_project_folder(
    config: &crate::config::Config,
    doc_hash: &str,
    source: &std::path::Path,
    filename: &str,
    ext: &str,
    word_count: usize,
) {
    let project_dir = config.project_dir(doc_hash);
    let outputs_dir = config.project_outputs_dir(doc_hash);

    if let Err(e) = tokio::fs::create_dir_all(&outputs_dir).await {
        tracing::warn!("project seed: create_dir {}: {e}", outputs_dir.display());
        return;
    }

    // original.{ext} — sólo si no existe aún (misma hash = mismo contenido)
    let ext_safe = if ext.is_empty() { "bin" } else { ext };
    let original = project_dir.join(format!("original.{ext_safe}"));
    if !original.exists() {
        if let Err(e) = tokio::fs::copy(source, &original).await {
            tracing::warn!("project seed: copy {}: {e}", original.display());
        }
    }

    // meta.json — sólo si no existe
    let meta_path = project_dir.join("meta.json");
    if !meta_path.exists() {
        let meta = serde_json::json!({
            "doc_hash":      doc_hash,
            "filename":      filename,
            "original_path": source.to_string_lossy(),
            "word_count":    word_count,
            "uploaded_at":   chrono::Utc::now().to_rfc3339(),
        });
        if let Err(e) = tokio::fs::write(&meta_path, meta.to_string().as_bytes()).await {
            tracing::warn!("project seed: write meta {}: {e}", meta_path.display());
        }
    }
}
