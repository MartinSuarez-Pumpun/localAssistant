use axum::{
    Router,
    extract::{Multipart, State},
    response::IntoResponse,
    routing::post,
    http::StatusCode,
};
use serde_json::json;
use tracing::info;

use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/upload", post(upload))
}

async fn upload(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let dir = state.config.uploads_dir();
    if let Err(e) = tokio::fs::create_dir_all(&dir).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("cannot create uploads dir: {e}"))
            .into_response();
    }

    let mut uploaded = Vec::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.file_name().unwrap_or("file").to_string();
        let data = match field.bytes().await {
            Ok(d) => d,
            Err(e) => {
                return (StatusCode::BAD_REQUEST, format!("read error: {e}")).into_response()
            }
        };

        let path = unique_path(&dir, &name);
        if let Err(e) = tokio::fs::write(&path, &data).await {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("write error: {e}"))
                .into_response();
        }

        let kind = detect_kind(&name);
        info!("upload: {} ({} B) → {}", name, data.len(), path.display());

        uploaded.push(json!({
            "name": name,
            "path": path.to_string_lossy(),
            "kind": kind,
        }));
    }

    axum::Json(uploaded).into_response()
}

fn unique_path(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }

    let stem = std::path::Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name);
    let ext = std::path::Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();

    for i in 1u32..100 {
        let p = dir.join(format!("{stem}_{i}{ext}"));
        if !p.exists() {
            return p;
        }
    }
    candidate
}

fn detect_kind(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "pdf" => "pdf",
        "docx" => "docx",
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "tiff" | "tif" | "svg" => "image",
        "txt" | "md" | "c" | "h" | "hpp" | "cpp" | "cc" | "cxx" | "py" | "js" | "ts"
        | "jsx" | "tsx" | "java" | "go" | "rs" | "rb" | "php" | "html" | "htm" | "css"
        | "json" | "yaml" | "yml" | "toml" | "xml" | "csv" | "sh" | "bash" | "zsh"
        | "lua" | "sql" | "swift" | "kt" | "dart" | "graphql" | "proto" | "tf" => "text",
        _ => "other",
    }
}