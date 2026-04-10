//! Bridge WebSocket con serve-rs.
//! El frontend envía/recibe mensajes JSON para invocar tools,
//! leer contexto, inyectar mensajes en el chat, etc.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

/// Mensaje enviado desde el plugin al host (serve-rs).
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", content = "payload")]
pub enum HostMsg {
    /// Invoca una tool del C core.
    ToolCall { name: String, args: serde_json::Value },
    /// Inyecta un mensaje en el chat.
    ChatInject { text: String },
    /// Lee el contexto actual.
    ContextGet,
}

/// Mensaje recibido desde serve-rs al plugin.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", content = "payload")]
pub enum PluginMsg {
    ToolResult { name: String, ok: bool, output: String },
    ContextData { messages: Vec<serde_json::Value> },
    Error { message: String },
}

/// Señal reactiva con el estado de conexión del bridge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BridgeState {
    Disconnected,
    Connected,
    Error,
}

// Por ahora el bridge es un stub — se implementa completamente
// cuando tengamos el endpoint WebSocket en serve-rs.
pub fn use_bridge() -> (ReadSignal<BridgeState>, impl Fn(HostMsg)) {
    let (state, _set_state) = signal(BridgeState::Disconnected);

    let send = move |_msg: HostMsg| {
        // TODO: enviar por WebSocket a ws://localhost:8080/bridge
        leptos::logging::log!("bridge: send (stub)");
    };

    (state, send)
}