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

    axum::Json(ExtractResponse { text, word_count, filename }).into_response()
}
