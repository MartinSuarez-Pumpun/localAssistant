use std::path::Path;
use axum::Router;
use tower_http::services::ServeDir;

/// Sirve `web/dist/` en `/` — cualquier ruta no capturada por otros handlers.
pub fn router(web_dist: &Path) -> Router<crate::AppState> {
    Router::new().fallback_service(
        ServeDir::new(web_dist).append_index_html_on_directories(true),
    )
}