// routes/publication.rs
// POST /api/publication — Pre-publication safety checks (SRS §14, PUB-001..PUB-005)
//
// Body: {
//   "text": "...",
//   "check_spelling": bool,
//   "check_tone":     bool,
//   "check_pii":      bool  // anonymization
// }
// Response: {
//   "ok": true,
//   "score": 0..100,
//   "ready": bool,
//   "issues": [{ "kind": "spelling|tone|pii", "severity": "low|medium|high", "message": "...", "snippet": "..." }],
//   "pii_found": [{ "type": "EMAIL|PHONE|DNI|IBAN|NAME", "match": "...", "suggestion": "..." }]
// }

use axum::{Router, extract::State, routing::post, response::IntoResponse, http::StatusCode};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::routes::crisis::call_llm_one_shot;

#[derive(Deserialize)]
pub struct PublicationRequest {
    pub text: String,
    #[serde(default = "default_true")]
    pub check_spelling: bool,
    #[serde(default = "default_true")]
    pub check_tone: bool,
    #[serde(default = "default_false")]
    pub check_pii: bool,
}

fn default_true()  -> bool { true }
fn default_false() -> bool { false }

#[derive(Serialize)]
pub struct Issue {
    pub kind:     String,
    pub severity: String,
    pub message:  String,
    pub snippet:  String,
}

#[derive(Serialize)]
pub struct PiiMatch {
    pub kind:       String,
    pub matched:    String,
    pub suggestion: String,
}

#[derive(Serialize)]
pub struct PublicationResponse {
    pub ok:        bool,
    pub score:     u32,
    pub ready:     bool,
    pub issues:    Vec<Issue>,
    pub pii_found: Vec<PiiMatch>,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/publication", post(publication_handler))
}

async fn publication_handler(
    State(state): State<AppState>,
    axum::Json(req): axum::Json<PublicationRequest>,
) -> impl IntoResponse {
    if req.text.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({
            "ok": false,
            "error": "Text is required."
        }))).into_response();
    }

    let mut issues:    Vec<Issue>    = Vec::new();
    let mut pii_found: Vec<PiiMatch> = Vec::new();

    // ── PII detection (regex-based, fast, deterministic) ─────────────────────
    if req.check_pii {
        pii_found = detect_pii(&req.text);
        for pii in &pii_found {
            issues.push(Issue {
                kind:     "pii".into(),
                severity: "high".into(),
                message:  format!("Detected {}: consider anonymising", pii.kind),
                snippet:  pii.matched.clone(),
            });
        }
    }

    // ── Spelling & tone checks via LLM (combined to save a round-trip) ───────
    if req.check_spelling || req.check_tone {
        let limited = if req.text.len() > 3000 { &req.text[..3000] } else { req.text.as_str() };
        let prompt = build_quality_prompt(limited, req.check_spelling, req.check_tone);
        if let Ok(content) = call_llm_one_shot(&state, &prompt).await {
            let clean = strip_code_fences(&content);
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&clean) {
                let empty = vec![];
                for item in json["issues"].as_array().unwrap_or(&empty) {
                    let kind     = item["kind"].as_str().unwrap_or("tone").to_string();
                    let severity = item["severity"].as_str().unwrap_or("low").to_string();
                    let message  = item["message"].as_str().unwrap_or("").to_string();
                    let snippet  = item["snippet"].as_str().unwrap_or("").to_string();
                    if !message.is_empty() {
                        issues.push(Issue { kind, severity, message, snippet });
                    }
                }
            }
        }
    }

    // ── Scoring: start at 100, deduct per issue by severity ──────────────────
    let mut score: i32 = 100;
    for iss in &issues {
        score -= match iss.severity.as_str() {
            "high"   => 15,
            "medium" => 8,
            _        => 3,
        };
    }
    let score = score.max(0) as u32;
    let ready = score >= 80 && !issues.iter().any(|i| i.severity == "high");

    let _ = state.db.log_event("publication", &serde_json::json!({
        "score": score,
        "ready": ready,
        "issue_count": issues.len(),
        "pii_count": pii_found.len(),
    }).to_string());

    axum::Json(PublicationResponse {
        ok: true,
        score,
        ready,
        issues,
        pii_found,
    }).into_response()
}

// ─── PII detection (regex patterns) ──────────────────────────────────────────

fn detect_pii(text: &str) -> Vec<PiiMatch> {
    let mut out = Vec::new();

    let patterns: &[(&str, &str, &str)] = &[
        ("EMAIL", r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}", "[EMAIL]"),
        ("PHONE", r"(?:\+?\d{1,3}[\s.-]?)?(?:\(?\d{2,3}\)?[\s.-]?)?\d{3}[\s.-]?\d{3,4}[\s.-]?\d{0,4}", "[PHONE]"),
        ("DNI",   r"\b\d{8}[A-Za-z]\b", "[DNI]"),
        ("IBAN",  r"\b[A-Z]{2}\d{2}[A-Z0-9]{10,30}\b", "[IBAN]"),
    ];

    for (kind, pattern, replacement) in patterns {
        if let Ok(re) = Regex::new(pattern) {
            for m in re.find_iter(text) {
                let matched = m.as_str().to_string();
                // Filter out phone false positives: must have at least 7 digits
                if *kind == "PHONE" {
                    let digits = matched.chars().filter(|c| c.is_ascii_digit()).count();
                    if digits < 7 { continue; }
                }
                out.push(PiiMatch {
                    kind:       kind.to_string(),
                    matched,
                    suggestion: replacement.to_string(),
                });
            }
        }
    }
    out
}

fn build_quality_prompt(text: &str, check_spelling: bool, check_tone: bool) -> String {
    let mut tasks = Vec::new();
    if check_spelling {
        tasks.push("- \"spelling\": errores ortográficos, tipográficos o de concordancia.");
    }
    if check_tone {
        tasks.push("- \"tone\": registro inadecuado para comunicación institucional (demasiado coloquial, agresivo, ambiguo, sesgado).");
    }
    let task_list = tasks.join("\n");

    format!(
        "Actúa como corrector editorial institucional. Analiza el TEXTO y devuelve SOLO un JSON \
         (sin markdown, sin preámbulos). Detecta únicamente problemas reales, no inventes. \
         Incluye como mucho 8 issues más relevantes.\n\n\
         Tipos de issue a detectar:\n{}\n\n\
         Cada issue: {{\"kind\": \"spelling|tone\", \"severity\": \"low|medium|high\", \
         \"message\": \"descripción breve\", \"snippet\": \"fragmento textual (máx 80 chars)\"}}.\n\n\
         Formato de salida: {{\"issues\": [ ... ]}}\n\n\
         TEXTO:\n{}",
        task_list, text
    )
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
