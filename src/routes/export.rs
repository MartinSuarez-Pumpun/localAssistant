use axum::{
    Router,
    extract::State,
    response::IntoResponse,
    routing::post,
    http::StatusCode,
};
use serde::Deserialize;
use tracing::info;

use crate::AppState;

#[derive(Deserialize)]
pub struct ExportRequest {
    /// Ruta absoluta del archivo a exportar (generado en ai_base).
    source: String,
    /// Directorio destino (pen drive u otra ubicación permitida).
    dest_dir: String,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/export", post(export_file))
}

async fn export_file(
    State(state): State<AppState>,
    axum::Json(req): axum::Json<ExportRequest>,
) -> impl IntoResponse {
    let src  = std::path::Path::new(&req.source);
    let dest_dir = std::path::Path::new(&req.dest_dir);

    // Seguridad: fuente debe estar en ai_base
    if !src.starts_with(&state.config.ai_base) {
        return (StatusCode::FORBIDDEN, "Fuente fuera de ai_base").into_response();
    }

    // Destino debe ser una ruta permitida (ai_base o disco externo)
    if !is_allowed_dest(dest_dir, &state.config) {
        return (StatusCode::FORBIDDEN, "Destino no permitido").into_response();
    }

    if !src.is_file() {
        return (StatusCode::NOT_FOUND, "Archivo origen no encontrado").into_response();
    }

    // Crear directorio destino si no existe
    if let Err(e) = tokio::fs::create_dir_all(dest_dir).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("No se pudo crear destino: {e}")).into_response();
    }

    let filename = match src.file_name().and_then(|n| n.to_str()) {
        Some(n) => n,
        None => return (StatusCode::BAD_REQUEST, "Nombre de archivo inválido").into_response(),
    };

    let dest_path = dest_dir.join(filename);

    match tokio::fs::copy(src, &dest_path).await {
        Ok(bytes) => {
            info!("export: {} → {} ({} bytes)", src.display(), dest_path.display(), bytes);
            axum::Json(serde_json::json!({
                "ok": true,
                "dest": dest_path.to_string_lossy()
            })).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Error copiando: {e}")).into_response(),
    }
}

fn is_allowed_dest(path: &std::path::Path, config: &crate::config::Config) -> bool {
    if path.starts_with(&config.ai_base) { return true; }

    #[cfg(target_os = "macos")]
    if path.starts_with("/Volumes") { return true; }

    #[cfg(target_os = "linux")]
    if path.starts_with("/media") || path.starts_with("/mnt") { return true; }

    false
}
