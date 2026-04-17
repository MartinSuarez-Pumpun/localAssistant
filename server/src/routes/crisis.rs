// routes/crisis.rs
// POST /api/crisis — Crisis communication generator (SRS §18, CRI-001..CRI-005)
//
// Body: { "scenario": "data_breach", "facts": "...", "position": "...", "tool": "speech|qa|timeline" }
// Response: { "ok": true, "output": "...", "tool": "speech" }

use axum::{Router, extract::State, routing::post, response::IntoResponse, http::StatusCode};
use serde::{Deserialize, Serialize};

use crate::AppState;

#[derive(Deserialize)]
pub struct CrisisRequest {
    pub scenario: String,
    pub facts:    String,
    pub position: String,
    pub tool:     String, // "speech" | "qa" | "timeline"
}

#[derive(Serialize)]
pub struct CrisisResponse {
    pub ok:     bool,
    pub output: String,
    pub tool:   String,
}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub ok:    bool,
    pub error: String,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/crisis", post(crisis_handler))
}

async fn crisis_handler(
    State(state): State<AppState>,
    axum::Json(req): axum::Json<CrisisRequest>,
) -> impl IntoResponse {
    if req.facts.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({
            "ok": false,
            "error": "Confirmed facts are required."
        }))).into_response();
    }

    let prompt = build_crisis_prompt(&req);

    match call_llm_one_shot(&state, &prompt).await {
        Ok(output) => {
            // Log to audit
            let payload = serde_json::json!({
                "scenario": req.scenario,
                "tool": req.tool,
                "facts_len": req.facts.len(),
            }).to_string();
            let _ = state.db.log_event("crisis", &payload);

            axum::Json(CrisisResponse {
                ok: true,
                output,
                tool: req.tool,
            }).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(ErrorResponse {
            ok: false,
            error: format!("LLM call failed: {}", e),
        })).into_response(),
    }
}

fn build_crisis_prompt(req: &CrisisRequest) -> String {
    let scenario_label = match req.scenario.as_str() {
        "data_breach"   => "Brecha de seguridad / filtración de datos",
        "legal"         => "Crisis legal / investigación judicial",
        "reputational"  => "Crisis reputacional",
        "operational"   => "Crisis operativa / interrupción de servicio",
        "health_safety" => "Emergencia sanitaria o de seguridad",
        "financial"     => "Crisis financiera",
        other           => other,
    };

    let tool_instructions = match req.tool.as_str() {
        "speech" => "Redacta un DISCURSO DE COMPARECENCIA pública (400-600 palabras). Estructura: \
                     apertura empática, reconocimiento de los hechos, acciones tomadas, compromiso \
                     de transparencia, cierre firme. Tono institucional pero humano. No uses frases \
                     vacías. Evita tecnicismos innecesarios.",
        "qa"     => "Genera 8-10 PREGUNTAS DIFÍCILES que previsiblemente hará la prensa en rueda \
                     de prensa, junto con una RESPUESTA DEFENSIVA y factual para cada una. \
                     Formato: Q1: [pregunta] / A1: [respuesta]. Las preguntas deben ser incisivas \
                     y cubrir: responsabilidades, cronología, impacto, medidas, futuro.",
        "timeline" => "Extrae y ordena cronológicamente los HECHOS CLAVE de esta crisis en formato \
                       timeline: cada punto con fecha/hora estimada + descripción concisa (1-2 líneas). \
                       Marca claramente lo que es hecho confirmado vs. lo que está en investigación.",
        _ => "Genera material de comunicación de crisis adaptado al escenario.",
    };

    format!(
        "Eres un asesor experto en comunicación de crisis institucional. Responde SOLO con el material \
         solicitado, sin preámbulos ni meta-comentarios.\n\n\
         ESCENARIO: {}\n\n\
         HECHOS CONFIRMADOS:\n{}\n\n\
         POSICIÓN OFICIAL:\n{}\n\n\
         TAREA: {}",
        scenario_label,
        req.facts.trim(),
        if req.position.trim().is_empty() { "(no especificada)" } else { req.position.trim() },
        tool_instructions,
    )
}

/// Generic one-shot LLM call (non-streaming). Returns the full content string.
pub async fn call_llm_one_shot(state: &AppState, prompt: &str) -> anyhow::Result<String> {
    let snapshot = state.settings.read().unwrap().clone();
    let endpoint = snapshot.llm_endpoint.clone();
    let model = if snapshot.llm_model.is_empty() {
        "llama3".to_string()
    } else {
        snapshot.llm_model.clone()
    };
    let api_key = snapshot.api_key.clone();

    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": model,
        "messages": [{ "role": "user", "content": prompt }],
        "stream": false,
    });

    let mut req_builder = client.post(format!("{endpoint}/v1/chat/completions")).json(&body);
    if !api_key.is_empty() {
        req_builder = req_builder.header("Authorization", format!("Bearer {api_key}"));
    }

    let response = req_builder.send().await?;
    let json: serde_json::Value = response.json().await?;
    let content = json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();

    if content.is_empty() {
        anyhow::bail!("Empty response from LLM");
    }
    Ok(content)
}
