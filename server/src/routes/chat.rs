use axum::{
    Router,
    extract::State,
    routing::post,
    response::{IntoResponse, Response},
    http::{StatusCode, HeaderMap, header},
    body::Body,
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{error, info};

use crate::AppState;

// ─── System prompt ────────────────────────────────────────────────────────────

const SYSTEM_PROMPT: &str = "\
You are a helpful assistant. \
Never use emojis, icons, or decorative symbols in your responses. \
Use plain text only. \
Be concise and direct.";

// ─── Tipos ────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    pub messages: Vec<Value>,
    pub model: Option<String>,
    pub tools: Option<Vec<Value>>,
    #[serde(default)]
    #[allow(dead_code)]
    pub stream: bool,
}

/// Evento SSE enviado al cliente (WASM/browser)
#[allow(dead_code)]
#[derive(Serialize)]
#[serde(tag = "event", content = "data")]
pub enum SseEvent {
    Token   { text: String },
    Reasoning { text: String },
    ToolStart { name: String, id: String },
    ToolResult { name: String, ok: bool, output: String },
    Done,
    Error { message: String },
}

// ─── Router ───────────────────────────────────────────────────────────────────

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/chat/stream", post(chat_stream))
}

// ─── Handler principal ────────────────────────────────────────────────────────

async fn chat_stream(
    State(state): State<AppState>,
    axum::Json(req): axum::Json<ChatRequest>,
) -> Response {
    let (tx, rx) = mpsc::channel::<String>(64);
    let config   = state.config.clone();
    let snapshot = state.settings.read().unwrap().clone();
    let endpoint = snapshot.llm_endpoint.clone();
    let model = req.model
        .filter(|m| !m.is_empty())
        .or_else(|| if snapshot.llm_model.is_empty() { None } else { Some(snapshot.llm_model.clone()) })
        .unwrap_or_else(|| "llama3".into());
    let api_key = snapshot.api_key.clone();

    let db = state.db.clone();

    // Extraer contexto para el audit log antes de mover req
    let doc_name: String = req.messages.iter()
        .find(|m| m["role"].as_str() == Some("system"))
        .and_then(|m| m["content"].as_str())
        .map(|s| s.chars().take(60).collect())
        .unwrap_or_else(|| "unknown".to_string());
    let snippet: String = req.messages.iter().rev()
        .find(|m| m["role"].as_str() == Some("user"))
        .and_then(|m| m["content"].as_str())
        .map(|s| s.chars().take(100).collect())
        .unwrap_or_default();

    tokio::spawn(async move {
        let mut messages = req.messages.clone();
        let tools = req.tools.clone().unwrap_or_default();

        // Tool loop — máx 5 rondas (igual que serve.py)
        for round in 0..5 {
            info!("chat round {round}");
            match llm_stream_round(
                &endpoint,
                &model,
                &api_key,
                &messages,
                &tools,
                tx.clone(),
            )
            .await
            {
                Ok(RoundResult::Done) => break,
                Ok(RoundResult::ToolCalls(calls)) => {
                    // Añadir assistant message con tool_calls
                    let assistant_msg = serde_json::json!({
                        "role": "assistant",
                        "content": null,
                        "tool_calls": calls.iter().map(|c| serde_json::json!({
                            "id": c.id,
                            "type": "function",
                            "function": { "name": c.name, "arguments": c.arguments }
                        })).collect::<Vec<_>>()
                    });
                    messages.push(assistant_msg);

                    // Ejecutar cada tool
                    for call in &calls {
                        let _ = tx.send(format!(
                            "event: tool_start\ndata: {}\n\n",
                            serde_json::to_string(&serde_json::json!({
                                "name": call.name,
                                "id": call.id
                            })).unwrap()
                        )).await;

                        let result = crate::tools::exec(
                            &call.name,
                            &call.arguments,
                            config.clone(),
                        ).await;

                        let (ok, output) = match result {
                            Ok(out) => (true, out),
                            Err(e) => (false, e.to_string()),
                        };

                        // Detectar si el tool creó un archivo (output: "creado: /path")
                        let file_info = if ok { created_file_info(&output) } else { None };

                        let mut result_json = serde_json::json!({
                            "name": call.name,
                            "ok": ok,
                            "output": output
                        });
                        if let Some(fi) = file_info {
                            result_json["file"] = fi;
                        }

                        let _ = tx.send(format!(
                            "event: tool_result\ndata: {}\n\n",
                            serde_json::to_string(&result_json).unwrap()
                        )).await;

                        messages.push(serde_json::json!({
                            "role": "tool",
                            "tool_call_id": call.id,
                            "content": output
                        }));
                    }
                }
                Err(e) => {
                    error!("LLM error: {e}");
                    let _ = tx.send(format!(
                        "event: error\ndata: {}\n\n",
                        serde_json::to_string(&serde_json::json!({"message": e.to_string()})).unwrap()
                    )).await;
                    break;
                }
            }
        }

        // Registrar en audit log
        let ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let payload = serde_json::json!({
            "doc_name": doc_name,
            "snippet":  snippet,
            "ts":       ts,
        }).to_string();
        let _ = db.log_event("chat", &payload);

        let _ = tx.send("event: done\ndata: {}\n\n".into()).await;
    });

    let stream = ReceiverStream::new(rx).map(|s| Ok::<_, std::convert::Infallible>(s));
    let body = Body::from_stream(stream);

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, "text/event-stream".parse().unwrap());
    headers.insert(header::CACHE_CONTROL, "no-cache".parse().unwrap());
    headers.insert("X-Accel-Buffering", "no".parse().unwrap());

    (StatusCode::OK, headers, body).into_response()
}

// ─── Tool call acumulado ──────────────────────────────────────────────────────

#[derive(Debug, Default, Clone)]
struct ToolCall {
    id: String,
    name: String,
    arguments: String,
}

enum RoundResult {
    Done,
    ToolCalls(Vec<ToolCall>),
}

// ─── Una ronda de streaming contra el LLM ────────────────────────────────────

async fn llm_stream_round(
    endpoint: &str,
    model: &str,
    api_key: &str,
    messages: &[Value],
    tools: &[Value],
    tx: mpsc::Sender<String>,
) -> anyhow::Result<RoundResult> {
    let client = reqwest::Client::new();

    // Inyectar system prompt si el cliente no mandó uno ya
    let messages_with_system: Vec<Value> = if messages.iter().any(|m| m["role"] == "system") {
        messages.to_vec()
    } else {
        let mut v = vec![serde_json::json!({
            "role": "system",
            "content": SYSTEM_PROMPT
        })];
        v.extend_from_slice(messages);
        v
    };

    let mut body = serde_json::json!({
        "model": model,
        "messages": messages_with_system,
        "stream": true,
    });
    if !tools.is_empty() {
        body["tools"] = serde_json::json!(tools);
    }

    let mut req = client
        .post(format!("{endpoint}/v1/chat/completions"))
        .json(&body);
    if !api_key.is_empty() {
        req = req.header("Authorization", format!("Bearer {api_key}"));
    }
    let resp = req.send().await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("LLM {status}: {text}");
    }

    let mut stream    = resp.bytes_stream();
    let mut buf       = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    let mut text_acc  = String::new();
    // Para vLLM/Qwen3: el razonamiento llega SIN <think> de apertura,
    // solo con </think> al final antes de la respuesta real.
    // Empezamos en modo "thinking" y salimos al ver </think>.
    // Si el stream termina sin </think> (llama.cpp, modelos sin thinking),
    // emitimos "promote_reasoning" para que el frontend mueva el contenido al chat.
    let mut in_think      = true;
    let mut has_think_end = false; // ¿vimos </think> en algún momento?

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        buf.push_str(&String::from_utf8_lossy(&chunk));

        // Procesar líneas completas
        while let Some(pos) = buf.find('\n') {
            let line = buf[..pos].trim().to_string();
            buf = buf[pos + 1..].to_string();

            if line.is_empty() || line == "data: [DONE]" {
                continue;
            }

            let json_str = line.strip_prefix("data: ").unwrap_or(&line);
            let Ok(val) = serde_json::from_str::<Value>(json_str) else {
                continue;
            };

            let delta = &val["choices"][0]["delta"];

            // Contenido de texto: separar razonamiento de respuesta real
            if let Some(text) = delta["content"].as_str() {
                if !text.is_empty() {
                    for (is_think, segment) in split_on_think_end(text, &mut in_think) {
                        if is_think {
                            let _ = tx.send(format!(
                                "event: reasoning\ndata: {}\n\n",
                                serde_json::to_string(&serde_json::json!({"text": segment})).unwrap()
                            )).await;
                        } else {
                            has_think_end = true;
                            text_acc.push_str(&segment);
                            let _ = tx.send(format!(
                                "event: token\ndata: {}\n\n",
                                serde_json::to_string(&serde_json::json!({"text": segment})).unwrap()
                            )).await;
                        }
                    }
                }
            }

            // Reasoning nativo en campo separado (DeepSeek / QwQ)
            if let Some(text) = delta["reasoning_content"].as_str() {
                if !text.is_empty() {
                    has_think_end = true; // campo nativo = no hay </think> inline
                    let _ = tx.send(format!(
                        "event: reasoning\ndata: {}\n\n",
                        serde_json::to_string(&serde_json::json!({"text": text})).unwrap()
                    )).await;
                }
            }

            // Tool calls nativos (formato OpenAI delta acumulativo)
            if let Some(calls) = delta["tool_calls"].as_array() {
                for tc in calls {
                    let idx = tc["index"].as_u64().unwrap_or(0) as usize;
                    while tool_calls.len() <= idx {
                        tool_calls.push(ToolCall::default());
                    }
                    let entry = &mut tool_calls[idx];
                    if let Some(id) = tc["id"].as_str() {
                        entry.id = id.to_string();
                    }
                    if let Some(name) = tc["function"]["name"].as_str() {
                        entry.name.push_str(name);
                    }
                    if let Some(args) = tc["function"]["arguments"].as_str() {
                        entry.arguments.push_str(args);
                    }
                }
            }
        }
    }

    // Si nunca vimos </think> ni reasoning_content nativo: el modelo no usa thinking.
    // El frontend acumuló todo en la reasoning_buf — pedirle que lo mueva al chat.
    if !has_think_end {
        let _ = tx.send("event: promote_reasoning\ndata: {}\n\n".to_string()).await;
    }

    // Formato nativo: si hay tool_calls válidos, usarlos
    if !tool_calls.is_empty() && tool_calls.iter().any(|c| !c.name.is_empty()) {
        return Ok(RoundResult::ToolCalls(tool_calls));
    }

    // Fallback: detectar tool calls en formato texto (modelos que no usan función nativa)
    if let Some(parsed) = parse_text_tool_calls(&text_acc) {
        if !parsed.is_empty() {
            return Ok(RoundResult::ToolCalls(parsed));
        }
    }

    Ok(RoundResult::Done)
}

// ─── Parser de tool calls en formato texto ───────────────────────────────────
//
// Soporta los formatos más comunes de modelos Ollama/Llama que no usan
// la API nativa de function calling:
//
//   <|tool_call|>call:read_file(path="/...")<|tool_call|>
//   <|tool_call|>{"name":"read_file","arguments":{...}}<|tool_call|>
//   {"name":"read_file","arguments":{...}}

fn parse_text_tool_calls(text: &str) -> Option<Vec<ToolCall>> {
    let mut calls = Vec::new();

    // Formato 1: <|tool_call|>...<|tool_call|>
    let mut search = text;
    while let Some(start) = search.find("<|tool_call|>") {
        let after = &search[start + "<|tool_call|>".len()..];
        let end = after.find("<|tool_call|>").unwrap_or(after.len());
        let inner = after[..end].trim();

        if let Some(tc) = parse_single_text_call(inner) {
            calls.push(tc);
        }

        search = if start + "<|tool_call|>".len() + end + "<|tool_call|>".len() < search.len() {
            &search[start + "<|tool_call|>".len() + end + "<|tool_call|>".len()..]
        } else {
            break;
        };
    }

    // Formato 2: JSON inline {"name":"...","arguments":{...}} si no hay marcadores
    if calls.is_empty() {
        let trimmed = text.trim();
        if trimmed.starts_with('{') {
            if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
                if let Some(name) = v["name"].as_str().or_else(|| v["function"].as_str()) {
                    let args = v.get("arguments")
                        .or_else(|| v.get("parameters"))
                        .cloned()
                        .unwrap_or(serde_json::json!({}));
                    calls.push(ToolCall {
                        id: format!("txt-{}", calls.len()),
                        name: name.to_string(),
                        arguments: args.to_string(),
                    });
                }
            }
        }
    }

    if calls.is_empty() { None } else { Some(calls) }
}

/// Parsea un fragmento individual: puede ser JSON `{"name":...}` o
/// texto tipo `call:read_file(path="...")` / `read_file({"path":"..."})`.
fn parse_single_text_call(inner: &str) -> Option<ToolCall> {
    // Intentar JSON primero
    if let Ok(v) = serde_json::from_str::<Value>(inner) {
        let name = v["name"].as_str().or_else(|| v["function"].as_str())?.to_string();
        let args = v.get("arguments")
            .or_else(|| v.get("parameters"))
            .cloned()
            .unwrap_or(serde_json::json!({}));
        return Some(ToolCall {
            id: "txt-0".into(),
            name,
            arguments: args.to_string(),
        });
    }

    // Formato "call:func_name(key="val",...)" o "func_name(key="val",...)"
    let inner = inner.strip_prefix("call:").unwrap_or(inner);
    let paren = inner.find('(')?;
    let name = inner[..paren].trim().to_string();
    let rest = inner[paren + 1..].trim_end_matches(')');

    // Intentar parsear el contenido como JSON object
    let args_json = if rest.starts_with('{') {
        rest.to_string()
    } else {
        // Convertir key="value",key2="value2" → {"key":"value","key2":"value2"}
        python_kwargs_to_json(rest)
    };

    Some(ToolCall {
        id: "txt-0".into(),
        name,
        arguments: args_json,
    })
}

/// Convierte `key="val", key2=123` a `{"key":"val","key2":123}`.
fn python_kwargs_to_json(kwargs: &str) -> String {
    let mut map = serde_json::Map::new();
    // Split conservador: comas que no estén dentro de comillas
    let mut depth = 0usize;
    let mut in_str = false;
    let mut escape = false;
    let mut pairs: Vec<String> = Vec::new();
    let mut cur = String::new();

    for ch in kwargs.chars() {
        if escape { escape = false; cur.push(ch); continue; }
        if ch == '\\' { escape = true; cur.push(ch); continue; }
        if ch == '"' || ch == '\'' { in_str = !in_str; }
        if !in_str {
            if ch == '(' || ch == '{' || ch == '[' { depth += 1; }
            if ch == ')' || ch == '}' || ch == ']' { depth = depth.saturating_sub(1); }
            if ch == ',' && depth == 0 { pairs.push(cur.trim().to_string()); cur.clear(); continue; }
        }
        cur.push(ch);
    }
    if !cur.trim().is_empty() { pairs.push(cur.trim().to_string()); }

    for pair in &pairs {
        if let Some(eq) = pair.find('=') {
            let key = pair[..eq].trim().trim_matches('"').trim_matches('\'').to_string();
            let val_str = pair[eq + 1..].trim();
            // Parsear valor
            let val = if let Ok(v) = serde_json::from_str::<Value>(val_str) {
                v
            } else {
                // Quitar comillas simples si las tiene
                let stripped = val_str.trim_matches('\'');
                Value::String(stripped.to_string())
            };
            map.insert(key, val);
        }
    }

    serde_json::Value::Object(map).to_string()
}

// ─── Detección de archivos creados ───────────────────────────────────────────

/// Si el output de un tool empieza con "creado: /ruta" o "escrito: /ruta",
/// devuelve metadata del archivo para mostrarlo como card descargable.
fn created_file_info(output: &str) -> Option<Value> {
    for prefix in &["creado: ", "escrito: "] {
        if let Some(path_str) = output.trim().strip_prefix(prefix) {
            let path = std::path::Path::new(path_str.trim());
            if path.exists() {
                let name = path.file_name()?.to_str()?.to_string();
                let ext  = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
                let kind = match ext.as_str() {
                    "pdf"  => "pdf",
                    "docx" => "docx",
                    "txt" | "md" => "text",
                    _ => "file",
                };
                return Some(serde_json::json!({
                    "name": name,
                    "path": path_str.trim(),
                    "kind": kind
                }));
            }
        }
    }
    None
}

// ─── Parser de razonamiento en streaming ─────────────────────────────────────
//
// vLLM/Qwen3 emite el razonamiento SIN tag <think> de apertura:
//   [razonamiento...]</think>\n[respuesta real]
//
// Empezamos con in_think=true y salimos al ver </think>.
// Los modelos sin thinking (llama.cpp) nunca emiten </think>, así que
// in_think permanece true y al final del stream se emite promote_reasoning.
//
// También soporta el formato estándar <think>...</think> (DeepSeek, etc.)
// aunque en ese caso <think> aparece como texto de razonamiento (inocuo).

pub(crate) fn split_on_think_end(text: &str, in_think: &mut bool) -> Vec<(bool, String)> {
    if !*in_think {
        return if text.is_empty() { vec![] } else { vec![(false, text.to_string())] };
    }

    // Buscamos </think> para salir del bloque de razonamiento
    match text.find("</think>") {
        Some(pos) => {
            let mut out = Vec::new();
            if pos > 0 {
                out.push((true, text[..pos].to_string()));
            }
            *in_think = false;
            // Saltar saltos de línea tras </think>
            let after = text[pos + "</think>".len()..].trim_start_matches('\n');
            if !after.is_empty() {
                out.push((false, after.to_string()));
            }
            out
        }
        None => vec![(true, text.to_string())],
    }
}

// ─── Helpers SSE ─────────────────────────────────────────────────────────────

#[allow(dead_code)]
fn event_name(ev: &SseEvent) -> &'static str {
    match ev {
        SseEvent::Token { .. }      => "token",
        SseEvent::Reasoning { .. }  => "reasoning",
        SseEvent::ToolStart { .. }  => "tool_start",
        SseEvent::ToolResult { .. } => "tool_result",
        SseEvent::Done              => "done",
        SseEvent::Error { .. }      => "error",
    }
}

#[allow(dead_code)]
fn event_data(ev: &SseEvent) -> Value {
    match ev {
        SseEvent::Token { text }           => serde_json::json!({"text": text}),
        SseEvent::Reasoning { text }       => serde_json::json!({"text": text}),
        SseEvent::ToolStart { name, id }   => serde_json::json!({"name": name, "id": id}),
        SseEvent::ToolResult { name, ok, output } => serde_json::json!({"name": name, "ok": ok, "output": output}),
        SseEvent::Done                     => serde_json::json!({}),
        SseEvent::Error { message }        => serde_json::json!({"message": message}),
    }
}