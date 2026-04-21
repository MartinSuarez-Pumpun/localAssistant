// workspace.rs — POST /api/workspace/save
// Persists an output text to ~/.local-ai/projects/{doc_hash}/outputs/{filename}
// (era ~/.local-ai/workspace/{doc_hash}/ antes de T23).

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::post,
};
use serde::{Deserialize, Serialize};

use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/workspace/save", post(save_output))
        .route("/api/project/reveal", post(reveal_project))
        .route("/api/project/delete", post(delete_project))
}

#[derive(Deserialize)]
struct SaveRequest {
    doc_hash: String,
    filename: String,
    content:  String,
}

#[derive(Serialize)]
struct SaveResponse {
    ok:   bool,
    path: String,
}

#[derive(Serialize)]
struct ErrorResponse {
    ok:    bool,
    error: String,
}

async fn save_output(
    State(state): State<AppState>,
    Json(body): Json<SaveRequest>,
) -> Result<(StatusCode, Json<SaveResponse>), (StatusCode, Json<ErrorResponse>)> {
    // Validate doc_hash: non-empty, only alphanumeric, underscores, hyphens
    if body.doc_hash.is_empty()
        || !body.doc_hash.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                ok:    false,
                error: "doc_hash must be non-empty and contain only alphanumeric chars, underscores, or hyphens".into(),
            }),
        ));
    }

    // Validate filename: non-empty, no '/' or ".."
    if body.filename.is_empty() || body.filename.contains('/') || body.filename.contains("..") {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                ok:    false,
                error: "filename must be non-empty and must not contain '/' or '..'".into(),
            }),
        ));
    }

    let dest_dir = state.config.project_outputs_dir(&body.doc_hash);

    tokio::fs::create_dir_all(&dest_dir).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                ok:    false,
                error: format!("cannot create project outputs dir: {e}"),
            }),
        )
    })?;

    let dest_file = dest_dir.join(&body.filename);

    tokio::fs::write(&dest_file, body.content.as_bytes()).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                ok:    false,
                error: format!("cannot write file: {e}"),
            }),
        )
    })?;

    Ok((
        StatusCode::OK,
        Json(SaveResponse {
            ok:   true,
            path: dest_file.to_string_lossy().into_owned(),
        }),
    ))
}

#[derive(Deserialize)]
struct RevealRequest {
    doc_hash: String,
}

async fn reveal_project(
    State(state): State<AppState>,
    Json(body): Json<RevealRequest>,
) -> Result<(StatusCode, Json<SaveResponse>), (StatusCode, Json<ErrorResponse>)> {
    if body.doc_hash.is_empty()
        || !body.doc_hash.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                ok:    false,
                error: "doc_hash inválido".into(),
            }),
        ));
    }

    let dir = state.config.project_dir(&body.doc_hash);
    if !dir.exists() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                ok:    false,
                error: format!("proyecto no encontrado: {}", dir.display()),
            }),
        ));
    }

    #[cfg(target_os = "macos")]
    { let _ = std::process::Command::new("open").arg(&dir).spawn(); }
    #[cfg(target_os = "linux")]
    { let _ = std::process::Command::new("xdg-open").arg(&dir).spawn(); }

    Ok((
        StatusCode::OK,
        Json(SaveResponse {
            ok:   true,
            path: dir.to_string_lossy().into_owned(),
        }),
    ))
}

#[derive(Deserialize)]
struct DeleteRequest {
    doc_hash: String,
}

async fn delete_project(
    State(state): State<AppState>,
    Json(body): Json<DeleteRequest>,
) -> Result<(StatusCode, Json<SaveResponse>), (StatusCode, Json<ErrorResponse>)> {
    if body.doc_hash.is_empty()
        || !body.doc_hash.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                ok:    false,
                error: "doc_hash inválido".into(),
            }),
        ));
    }

    let dir = state.config.project_dir(&body.doc_hash);
    let path_str = dir.to_string_lossy().into_owned();

    if !dir.exists() {
        return Ok((
            StatusCode::OK,
            Json(SaveResponse { ok: true, path: path_str }),
        ));
    }

    tokio::fs::remove_dir_all(&dir).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                ok:    false,
                error: format!("no se pudo borrar {}: {e}", dir.display()),
            }),
        )
    })?;

    tracing::info!("project delete: {}", path_str);

    Ok((
        StatusCode::OK,
        Json(SaveResponse { ok: true, path: path_str }),
    ))
}
