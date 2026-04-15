/**
 * history.rs
 *
 * GET /api/transformations   → últimas 50 transformaciones
 * GET /api/documents         → documentos únicos recientes
 * GET /api/audit             → últimas 100 entradas del log de auditoría
 * DELETE /api/purge          → borrado de emergencia (SEC-002, requiere confirmación)
 */

use axum::{
    Router,
    extract::{Query, State},
    response::IntoResponse,
    routing::{delete, get},
    http::StatusCode,
};
use serde::Deserialize;
use tracing::info;

use crate::AppState;

#[derive(Deserialize)]
pub struct LimitQuery {
    #[serde(default = "default_limit")]
    limit: u32,
}
fn default_limit() -> u32 { 50 }

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/transformations", get(list_transformations))
        .route("/api/documents",       get(list_documents))
        .route("/api/audit",           get(list_audit))
        .route("/api/purge",           delete(purge_all))
}

// ── Handlers ──────────────────────────────────────────────────────────────────

async fn list_transformations(
    State(state): State<AppState>,
    Query(q):     Query<LimitQuery>,
) -> impl IntoResponse {
    match state.db.list_transformations(q.limit.min(200)) {
        Ok(rows) => axum::Json(serde_json::json!({ "ok": true, "data": rows })).into_response(),
        Err(e)   => (StatusCode::INTERNAL_SERVER_ERROR,
                     axum::Json(serde_json::json!({ "ok": false, "error": e.to_string() }))).into_response(),
    }
}

async fn list_documents(
    State(state): State<AppState>,
    Query(q):     Query<LimitQuery>,
) -> impl IntoResponse {
    match state.db.list_recent_documents(q.limit.min(100)) {
        Ok(rows) => axum::Json(serde_json::json!({ "ok": true, "data": rows })).into_response(),
        Err(e)   => (StatusCode::INTERNAL_SERVER_ERROR,
                     axum::Json(serde_json::json!({ "ok": false, "error": e.to_string() }))).into_response(),
    }
}

async fn list_audit(
    State(state): State<AppState>,
    Query(q):     Query<LimitQuery>,
) -> impl IntoResponse {
    match state.db.list_audit(q.limit.min(500)) {
        Ok(rows) => axum::Json(serde_json::json!({ "ok": true, "data": rows })).into_response(),
        Err(e)   => (StatusCode::INTERNAL_SERVER_ERROR,
                     axum::Json(serde_json::json!({ "ok": false, "error": e.to_string() }))).into_response(),
    }
}

/// SEC-002 — Expurgo de emergencia: elimina todos los datos almacenados.
/// Requiere header `X-Confirm: PURGE` para evitar borrados accidentales.
async fn purge_all(
    State(state): State<AppState>,
    headers:      axum::http::HeaderMap,
) -> impl IntoResponse {
    let confirm = headers.get("x-confirm")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if confirm != "PURGE" {
        return (StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "ok": false,
                "error": "Falta header X-Confirm: PURGE"
            }))).into_response();
    }

    let db = &state.db;
    let r1 = db.log_event("purge", r#"{"note":"emergency purge initiated"}"#);

    // Borrar tablas
    {
        let conn = db.0.lock().unwrap();
        let _ = conn.execute_batch("DELETE FROM transformations; DELETE FROM audit_log;");
    }

    info!("PURGE ejecutado (SEC-002)");
    let _ = r1; // log ya no importa tras el purge
    axum::Json(serde_json::json!({ "ok": true, "message": "Datos eliminados" })).into_response()
}
