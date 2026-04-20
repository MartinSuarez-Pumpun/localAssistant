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

// ─── Tipos ────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct AutostartStatus {
    enabled: bool,
    asked:   bool,
}

#[derive(Deserialize)]
struct AutostartRequest {
    enabled: bool,
}

// ─── Rutas ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/autostart", get(get_autostart).post(set_autostart))
}

// Ruta al .desktop de autostart del usuario (XDG)
fn autostart_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|d| d.join("autostart").join("localaiassistant.desktop"))
}

async fn get_autostart(State(state): State<AppState>) -> impl IntoResponse {
    let enabled = autostart_path().map(|p| p.exists()).unwrap_or(false);
    let asked   = state.settings.read().unwrap().autostart_asked;
    Json(AutostartStatus { enabled, asked })
}

async fn set_autostart(
    State(state): State<AppState>,
    Json(body): Json<AutostartRequest>,
) -> impl IntoResponse {
    // Marcar como "ya preguntado" en settings independientemente de la respuesta
    {
        let mut settings = state.settings.write().unwrap();
        settings.autostart_asked = true;
        let path = state.config.settings_path();
        if let Err(e) = settings.save(&path) {
            warn!("Error guardando settings tras autostart: {e}");
        }
    }

    let Some(autostart_path) = autostart_path() else {
        return (StatusCode::NOT_IMPLEMENTED, "autostart no soportado en esta plataforma")
            .into_response();
    };

    if body.enabled {
        // Construir la entrada .desktop apuntando al ejecutable actual
        let exe = match std::env::current_exe() {
            Ok(p)  => p,
            Err(e) => {
                warn!("current_exe() falló: {e}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };
        let web_dist    = state.config.web_dist.display().to_string();
        let plugins_dir = state.config.plugins_dir.display().to_string();

        // Citar rutas que puedan tener espacios
        let exec_line = format!(
            "\"{}\" --web-dist \"{}\" --plugins-dir \"{}\" --kiosk",
            exe.display(), web_dist, plugins_dir
        );

        let content = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=LocalAI Assistant\n\
             Exec={exec_line}\n\
             Hidden=false\n\
             NoDisplay=false\n\
             X-GNOME-Autostart-enabled=true\n"
        );

        if let Some(parent) = autostart_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                warn!("Error creando directorio autostart: {e}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        }
        if let Err(e) = std::fs::write(&autostart_path, content) {
            warn!("Error escribiendo autostart: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    } else if autostart_path.exists() {
        if let Err(e) = std::fs::remove_file(&autostart_path) {
            warn!("Error eliminando autostart: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    let enabled = autostart_path.exists();
    Json(AutostartStatus { enabled, asked: true }).into_response()
}
