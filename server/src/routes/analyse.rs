// routes/analyse.rs
// POST /api/analyse – analyzes document text for readability, sentiment, and entities.
// Body: { "text": "...", "include_sentiment": bool, "include_ner": bool }
// Returns: { "flesch_score": f32, "grade_level": f32, "word_count": u32, "sentence_count": u32, ... }

use axum::{Router, extract::State, routing::post, response::IntoResponse, http::StatusCode};
use serde::{Deserialize, Serialize};

use crate::AppState;

#[derive(Deserialize)]
pub struct AnalyseRequest {
    pub text: String,
    #[serde(default)]
    pub include_sentiment: bool,
    #[serde(default)]
    pub include_ner: bool,
}

#[derive(Serialize)]
pub struct AnalyseResponse {
    pub flesch_score: f32,
    pub grade_level: f32,
    pub word_count: u32,
    pub sentence_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sentiment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sentiment_score: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entities: Option<Vec<Entity>>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Entity {
    pub text: String,
    #[serde(rename = "type")]
    pub entity_type: String,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/analyse", post(analyse_text))
}

async fn analyse_text(
    State(state): State<AppState>,
    axum::Json(req): axum::Json<AnalyseRequest>,
) -> impl IntoResponse {
    let text = &req.text;

    // Calculate Flesch-Kincaid metrics
    let (flesch_score, grade_level, word_count, sentence_count) = calculate_readability(text);

    let mut response = AnalyseResponse {
        flesch_score,
        grade_level,
        word_count,
        sentence_count,
        sentiment: None,
        sentiment_score: None,
        entities: None,
    };

    // T02 - Add sentiment scoring via LLM
    if req.include_sentiment {
        if let Ok((sentiment, score)) = call_llm_sentiment(&state, text).await {
            response.sentiment = Some(sentiment);
            response.sentiment_score = Some(score);
        }
    }

    // T03 - Add NER extraction via LLM
    if req.include_ner {
        if let Ok(entities) = call_llm_ner(&state, text).await {
            response.entities = Some(entities);
        }
    }

    axum::Json(response).into_response()
}

// ─── Flesch-Kincaid Readability Calculation ────────────────────────────────────

fn calculate_readability(text: &str) -> (f32, f32, u32, u32) {
    let words = text.split_whitespace().collect::<Vec<_>>();
    let word_count = words.len() as u32;

    // Count sentences (simplified: ends with . ! ?)
    let sentence_count = text.matches(|c| c == '.' || c == '!' || c == '?').count() as u32;
    let sentence_count = if sentence_count == 0 { 1 } else { sentence_count };

    // Count syllables (English heuristic)
    let syllable_count = count_syllables(&words);

    // Flesch Reading Ease formula:
    // 206.835 - 1.015(words/sentences) - 84.6(syllables/words)
    let flesch_score = if word_count > 0 && sentence_count > 0 {
        206.835
            - 1.015 * (word_count as f32 / sentence_count as f32)
            - 84.6 * (syllable_count as f32 / word_count as f32)
    } else {
        0.0
    };

    // Flesch-Kincaid Grade Level formula:
    // 0.39(words/sentences) + 11.8(syllables/words) - 15.59
    let grade_level = if word_count > 0 && sentence_count > 0 {
        0.39 * (word_count as f32 / sentence_count as f32)
            + 11.8 * (syllable_count as f32 / word_count as f32)
            - 15.59
    } else {
        0.0
    };

    // Clamp scores to reasonable ranges
    let flesch_score = flesch_score.max(0.0).min(100.0);
    let grade_level = grade_level.max(0.0);

    (flesch_score, grade_level, word_count, sentence_count)
}

// ─── Syllable Counter (English Heuristic) ──────────────────────────────────────
// Simplified approach: count vowel groups, subtract silent e's, etc.

fn count_syllables(words: &[&str]) -> u32 {
    let mut total = 0;
    for word in words {
        total += count_syllables_in_word(word);
    }
    total.max(1) // At least 1 syllable per word
}

fn count_syllables_in_word(word: &str) -> u32 {
    let word = word.to_lowercase();
    let word = word.trim_matches(|c: char| !c.is_alphabetic());

    if word.is_empty() {
        return 0;
    }

    let mut syllable_count: u32 = 0;
    let mut previous_was_vowel = false;
    let vowels = "aeiouy";

    for ch in word.chars() {
        let is_vowel = vowels.contains(ch);
        if is_vowel && !previous_was_vowel {
            syllable_count += 1;
        }
        previous_was_vowel = is_vowel;
    }

    // Adjust for silent e
    if word.ends_with('e') {
        syllable_count = syllable_count.saturating_sub(1);
    }

    // Adjust for le at end
    if word.ends_with("le") && word.len() > 2 {
        let before_le = &word[..word.len() - 2];
        if !before_le.ends_with(|c: char| vowels.contains(c)) {
            syllable_count += 1;
        }
    }

    syllable_count.max(1)
}

// ─── LLM NER Extraction ───────────────────────────────────────────────────────

async fn call_llm_ner(state: &crate::AppState, text: &str) -> anyhow::Result<Vec<Entity>> {
    let snapshot = state.settings.read().unwrap().clone();
    let endpoint = snapshot.llm_endpoint.clone();
    let model = if snapshot.llm_model.is_empty() {
        "llama3".to_string()
    } else {
        snapshot.llm_model.clone()
    };
    let api_key = snapshot.api_key.clone();

    // Limit input to 2000 chars to avoid token overflow
    let limited_text = if text.len() > 2000 {
        &text[..2000]
    } else {
        text
    };

    let prompt = format!(
        "Extract named entities from this text. Respond ONLY with valid JSON (no markdown, no extra text):\n\
         {{\n  \"entities\": [\n    {{\"text\": \"entity name\", \"type\": \"PERSON|ORG|PLACE|DATE|OTHER\"}}\n  ]\n}}\n\n\
         Text: {}",
        limited_text
    );

    let messages = vec![serde_json::json!({
        "role": "user",
        "content": prompt
    })];

    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": model,
        "messages": messages,
        "stream": false,
    });

    let mut req_builder = client.post(format!("{endpoint}/v1/chat/completions")).json(&body);
    if !api_key.is_empty() {
        req_builder = req_builder.header("Authorization", format!("Bearer {api_key}"));
    }

    let response = req_builder.send().await?;
    let json: serde_json::Value = response.json().await?;

    // Extract content from response
    let content = json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();

    // Parse JSON response
    let ner_json: serde_json::Value = serde_json::from_str(&content)?;
    let empty_vec = vec![];
    let entities_arr = ner_json["entities"].as_array().unwrap_or(&empty_vec);

    let entities: Vec<Entity> = entities_arr
        .iter()
        .filter_map(|e| {
            let text = e["text"].as_str()?.to_string();
            let entity_type = e["type"].as_str()?.to_string();
            Some(Entity { text, entity_type })
        })
        .collect();

    Ok(entities)
}

// ─── LLM Sentiment Analysis ────────────────────────────────────────────────────

async fn call_llm_sentiment(state: &crate::AppState, text: &str) -> anyhow::Result<(String, f32)> {
    let snapshot = state.settings.read().unwrap().clone();
    let endpoint = snapshot.llm_endpoint.clone();
    let model = if snapshot.llm_model.is_empty() {
        "llama3".to_string()
    } else {
        snapshot.llm_model.clone()
    };
    let api_key = snapshot.api_key.clone();

    let prompt = format!(
        "Analyze the sentiment of this text. Respond ONLY with valid JSON (no markdown, no extra text):\n\
         {{\n  \"sentiment\": \"positive\" | \"neutral\" | \"negative\",\n  \"score\": <float from -1.0 to 1.0>\n}}\n\n\
         Text: {}",
        text
    );

    let messages = vec![serde_json::json!({
        "role": "user",
        "content": prompt
    })];

    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": model,
        "messages": messages,
        "stream": false,
    });

    let mut req_builder = client.post(format!("{endpoint}/v1/chat/completions")).json(&body);
    if !api_key.is_empty() {
        req_builder = req_builder.header("Authorization", format!("Bearer {api_key}"));
    }

    let response = req_builder.send().await?;
    let json: serde_json::Value = response.json().await?;

    // Extract content from response
    let content = json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();

    // Parse JSON response
    let sentiment_json: serde_json::Value = serde_json::from_str(&content)?;
    let sentiment = sentiment_json["sentiment"]
        .as_str()
        .unwrap_or("neutral")
        .to_string();
    let score = sentiment_json["score"].as_f64().unwrap_or(0.0) as f32;

    Ok((sentiment, score))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_syllable_counting() {
        assert_eq!(count_syllables_in_word("hello"), 2); // hel-lo
        assert_eq!(count_syllables_in_word("beautiful"), 3); // beau-ti-ful
        assert_eq!(count_syllables_in_word("cat"), 1); // cat
    }

    #[test]
    fn test_readability() {
        let text = "The quick brown fox jumps over the lazy dog. This is a test sentence.";
        let (flesch, grade, words, sentences) = calculate_readability(text);
        assert!(flesch >= 0.0 && flesch <= 100.0);
        assert!(grade >= 0.0);
        assert_eq!(words, 14);
        assert_eq!(sentences, 2);
    }
}
