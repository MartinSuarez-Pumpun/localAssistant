// routes/audit.rs
// GET /api/audit – returns recent audit log entries from database

use axum::{Router, extract::State, routing::get, response::IntoResponse};
use serde::Serialize;

use crate::AppState;

#[derive(Serialize)]
pub struct AuditResponse {
    pub id: i64,
    pub event_type: String,
    pub payload: String,
    pub ts: String,
}

#[derive(Serialize)]
pub struct ApiResponse<T> {
    pub ok: bool,
    pub data: Vec<T>,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/audit", get(list_audit))
}

async fn list_audit(
    State(state): State<AppState>,
) -> impl IntoResponse {
    match state.db.list_audit(50) {
        Ok(rows) => {
            let data: Vec<AuditResponse> = rows
                .into_iter()
                .map(|r| AuditResponse {
                    id: r.id,
                    event_type: r.event_type,
                    payload: r.payload,
                    ts: r.ts,
                })
                .collect();
            axum::Json(ApiResponse {
                ok: true,
                data,
            }).into_response()
        }
        Err(_) => axum::Json(ApiResponse::<AuditResponse> {
            ok: false,
            data: vec![],
        }).into_response(),
    }
}
