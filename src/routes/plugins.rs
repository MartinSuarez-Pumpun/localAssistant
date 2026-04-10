use std::path::Path;

use axum::{
    Router,
    extract::{Path as AxumPath, State},
    routing::get,
    response::{IntoResponse, Response},
    http::{StatusCode, header},
    body::Body,
};
use tracing::warn;

use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/plugins/",                      get(list_plugins))
        .route("/plugins/:id/manifest",         get(plugin_manifest))
        .route("/plugins/:id/app",             get(plugin_app_index))
        .route("/plugins/:id/app/",             get(plugin_app_index))
        .route("/plugins/:id/app/*path",      get(plugin_app_file))
}

// ─── Lista de plugins instalados ──────────────────────────────────────────────

async fn list_plugins(State(state): State<AppState>) -> impl IntoResponse {
    let dir = &state.config.plugins_dir;
    let mut plugins = vec![];

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let manifest = entry.path().join("plugin.json");
            if let Ok(raw) = std::fs::read_to_string(&manifest) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
                    plugins.push(val);
                }
            }
        }
    }

    axum::Json(plugins)
}

// ─── Manifest de un plugin ────────────────────────────────────────────────────

async fn plugin_manifest(
    AxumPath(id): AxumPath<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let manifest = state.config.plugins_dir.join(&id).join("plugin.json");

    match std::fs::read_to_string(&manifest) {
        Ok(raw) => match serde_json::from_str::<serde_json::Value>(&raw) {
            Ok(val) => axum::Json(val).into_response(),
            Err(e) => {
                warn!("plugin {id}: JSON inválido: {e}");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        },
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

// ─── Archivos estáticos del plugin (app web) ──────────────────────────────────

async fn plugin_app_index(
    AxumPath(id): AxumPath<String>,
    State(state): State<AppState>,
) -> Response {
    serve_plugin_file(&state.config.plugins_dir, &id, "index.html").await
}

async fn plugin_app_file(
    AxumPath((id, path)): AxumPath<(String, String)>,
    State(state): State<AppState>,
) -> Response {
    let file_path = state.config.plugins_dir.join(&id).join("app").join(&path);

    match tokio::fs::read(&file_path).await {
        Ok(contents) => {
            let mime = mime_for(&file_path);
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime)
                .body(Body::from(contents))
                .unwrap()
        }
        Err(_) => {
            // SPA fallback para rutas sin extensión
            if file_path.extension().is_none() {
                serve_plugin_file(&state.config.plugins_dir, &id, "index.html").await
            } else {
                StatusCode::NOT_FOUND.into_response()
            }
        }
    }
}

async fn serve_plugin_file(plugins_dir: &Path, id: &str, path: &str) -> Response {
    let file_path = plugins_dir.join(id).join("app").join(path);

    match tokio::fs::read(&file_path).await {
        Ok(contents) => {
            let mime = mime_for(&file_path);
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime)
                .body(Body::from(contents))
                .unwrap()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

fn mime_for(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html")        => "text/html; charset=utf-8",
        Some("js" | "mjs") => "application/javascript",
        Some("wasm")        => "application/wasm",
        Some("css")         => "text/css",
        Some("json")        => "application/json",
        Some("png")         => "image/png",
        Some("svg")         => "image/svg+xml",
        Some("ico")         => "image/x-icon",
        Some("txt")         => "text/plain; charset=utf-8",
        _                   => "application/octet-stream",
    }
}
