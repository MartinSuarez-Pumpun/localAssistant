// routes/readability.rs — POST /api/readability

use axum::{Router, routing::post, response::IntoResponse};
use serde::{Deserialize, Serialize};
use crate::tools::readability as rl;
use crate::AppState;

#[derive(Deserialize)]
struct ReadabilityRequest {
    text: String,
}

#[derive(Serialize)]
struct ReadabilityResponse {
    ok: bool,
    score: f32,
    grade: String,
    word_count: u32,
    sentence_count: u32,
    syllable_count: u32,
}

async fn readability_handler(
    axum::Json(req): axum::Json<ReadabilityRequest>,
) -> impl IntoResponse {
    let result = rl::analyse(&req.text);
    axum::Json(ReadabilityResponse {
        ok: true,
        score: result.score,
        grade: result.grade,
        word_count: result.word_count,
        sentence_count: result.sentence_count,
        syllable_count: result.syllable_count,
    })
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/readability", post(readability_handler))
}
