use axum::{
    Router,
    routing::get,
    extract::State,
    response::IntoResponse,
    Json,
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::AppState;

// ─── Modelo ───────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
pub struct Settings {
    pub llm_endpoint: String,
    pub llm_model:    String,
    pub api_key:      String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            llm_endpoint: "http://localhost:11434".into(),
            llm_model:    String::new(),
            api_key:      String::new(),
        }
    }
}

impl Settings {
    pub fn load(path: &std::path::Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &std::path::Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

// ─── Rutas ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/settings", get(get_settings).post(save_settings))
}

async fn get_settings(State(state): State<AppState>) -> impl IntoResponse {
    let s = state.settings.read().unwrap().clone();
    Json(s)
}

async fn save_settings(
    State(state): State<AppState>,
    Json(body): Json<Settings>,
) -> impl IntoResponse {
    let path = state.config.settings_path();
    if let Err(e) = body.save(&path) {
        warn!("Error guardando settings: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    *state.settings.write().unwrap() = body.clone();
    Json(body).into_response()
}
