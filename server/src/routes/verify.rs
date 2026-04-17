// routes/verify.rs
// POST /api/verify — Verifiability analysis: classifies claims in generated output
// against the source document (SRS §13, VER-001..VER-005)
//
// Body: { "source": "...", "output": "..." }
// Response: { "ok": true, "claims": [{ "text": "...", "category": "VERIFIED|INFERRED|NO_SOURCE", "note": "..." }] }

use axum::{Router, extract::State, routing::post, response::IntoResponse, http::StatusCode};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::routes::crisis::call_llm_one_shot;

#[derive(Deserialize)]
pub struct VerifyRequest {
    pub source: String,
    pub output: String,
}

#[derive(Serialize)]
pub struct Claim {
    pub text:     String,
    pub category: String, // VERIFIED | INFERRED | NO_SOURCE
    pub note:     String,
}

#[derive(Serialize)]
pub struct VerifyResponse {
    pub ok:     bool,
    pub claims: Vec<Claim>,
    pub verified_count: usize,
    pub inferred_count: usize,
    pub no_source_count: usize,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/verify", post(verify_handler))
}

async fn verify_handler(
    State(state): State<AppState>,
    axum::Json(req): axum::Json<VerifyRequest>,
) -> impl IntoResponse {
    if req.source.trim().is_empty() || req.output.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({
            "ok": false,
            "error": "Both source and output are required."
        }))).into_response();
    }

    // Limit input sizes to keep token usage sane
    let source = truncate(&req.source, 4000);
    let output = truncate(&req.output, 2000);

    let prompt = format!(
        "Eres un auditor de verificabilidad. Divide el TEXTO GENERADO en afirmaciones concretas \
         (claims) y clasifica cada una comparándola con el DOCUMENTO FUENTE:\n\
         - VERIFIED: la afirmación está literal o semánticamente presente en el documento fuente.\n\
         - INFERRED: es una conclusión lógica razonable a partir del fuente, aunque no aparece literal.\n\
         - NO_SOURCE: el modelo la añadió sin base en el documento fuente (posible alucinación).\n\n\
         Responde SOLO con JSON válido (sin markdown, sin texto extra):\n\
         {{\"claims\": [{{\"text\": \"afirmación\", \"category\": \"VERIFIED\", \"note\": \"explicación breve\"}}]}}\n\n\
         DOCUMENTO FUENTE:\n{}\n\n\
         TEXTO GENERADO:\n{}",
        source, output
    );

    match call_llm_one_shot(&state, &prompt).await {
        Ok(content) => {
            // Strip potential markdown code fences
            let clean = strip_code_fences(&content);
            match serde_json::from_str::<serde_json::Value>(&clean) {
                Ok(json) => {
                    let empty = vec![];
                    let arr = json["claims"].as_array().unwrap_or(&empty);
                    let claims: Vec<Claim> = arr.iter().filter_map(|c| {
                        let text = c["text"].as_str()?.to_string();
                        let category = c["category"].as_str()?.to_string().to_uppercase();
                        let note = c["note"].as_str().unwrap_or("").to_string();
                        let category = match category.as_str() {
                            "VERIFIED" | "INFERRED" | "NO_SOURCE" => category,
                            _ => "NO_SOURCE".to_string(),
                        };
                        Some(Claim { text, category, note })
                    }).collect();

                    let verified  = claims.iter().filter(|c| c.category == "VERIFIED").count();
                    let inferred  = claims.iter().filter(|c| c.category == "INFERRED").count();
                    let no_source = claims.iter().filter(|c| c.category == "NO_SOURCE").count();

                    let _ = state.db.log_event("verify", &serde_json::json!({
                        "total_claims": claims.len(),
                        "verified": verified,
                        "inferred": inferred,
                        "no_source": no_source,
                    }).to_string());

                    axum::Json(VerifyResponse {
                        ok: true,
                        claims,
                        verified_count:  verified,
                        inferred_count:  inferred,
                        no_source_count: no_source,
                    }).into_response()
                }
                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({
                    "ok": false,
                    "error": format!("Failed to parse LLM response as JSON: {}", e),
                    "raw": content,
                }))).into_response(),
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({
            "ok": false,
            "error": format!("LLM call failed: {}", e),
        }))).into_response(),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() } else { format!("{}…", &s[..max]) }
}

fn strip_code_fences(s: &str) -> String {
    let t = s.trim();
    if t.starts_with("```") {
        let without_open = t.trim_start_matches("```json").trim_start_matches("```").trim();
        if let Some(idx) = without_open.rfind("```") {
            return without_open[..idx].trim().to_string();
        }
    }
    t.to_string()
}
