// routes/transformations.rs
// GET /api/transformations – returns recent transformations from database

use axum::{Router, extract::State, routing::get, response::IntoResponse};
use serde::Serialize;

use crate::AppState;

#[derive(Serialize)]
pub struct TransformationResponse {
    pub id: i64,
    pub doc_name: String,
    pub action: String,
    pub word_count: u32,
    pub created_at: String,
}

#[derive(Serialize)]
pub struct ApiResponse<T> {
    pub ok: bool,
    pub data: Vec<T>,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/transformations", get(list_transformations))
}

async fn list_transformations(
    State(state): State<AppState>,
) -> impl IntoResponse {
    match state.db.list_transformations(20) {
        Ok(rows) => {
            let data: Vec<TransformationResponse> = rows
                .into_iter()
                .map(|r| TransformationResponse {
                    id: r.id,
                    doc_name: r.doc_name,
                    action: r.action,
                    word_count: r.word_count,
                    created_at: r.created_at,
                })
                .collect();
            axum::Json(ApiResponse {
                ok: true,
                data,
            }).into_response()
        }
        Err(_) => axum::Json(ApiResponse::<TransformationResponse> {
            ok: false,
            data: vec![],
        }).into_response(),
    }
}
