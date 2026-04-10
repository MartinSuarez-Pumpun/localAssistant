use axum::{
    Router,
    extract::{Query, State},
    response::IntoResponse,
    routing::get,
    http::{StatusCode, HeaderMap, header},
};
use serde::Deserialize;

use crate::AppState;

#[derive(Deserialize)]
pub struct DownloadParams {
    path: String,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/download", get(download))
}

async fn download(
    State(state): State<AppState>,
    Query(params): Query<DownloadParams>,
) -> impl IntoResponse {
    let requested = std::path::Path::new(&params.path);

    // Seguridad: solo servir archivos dentro de ai_base
    if !requested.starts_with(&state.config.ai_base) {
        return StatusCode::FORBIDDEN.into_response();
    }

    let data = match tokio::fs::read(requested).await {
        Ok(d) => d,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };

    let filename = requested
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");

    let content_type = match requested.extension().and_then(|e| e.to_str()) {
        Some("pdf")  => "application/pdf",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("txt") | Some("md") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    };

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, content_type.parse().unwrap());
    headers.insert(
        header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{filename}\"").parse().unwrap(),
    );

    (StatusCode::OK, headers, data).into_response()
}
