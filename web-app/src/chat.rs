use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use crate::file_browser::FileBrowserCtx;

// ─── Tipos ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub enum Role { User, Assistant, Tool }

#[derive(Clone, Debug)]
pub struct Message {
    pub role:        Role,
    pub content:     String,
    pub tool:        Option<String>,
    pub ok:          Option<bool>,
    pub attachments: Vec<AttachmentInfo>,  // para burbujas de usuario
    pub file:        Option<FileResult>,   // para tool results con archivo
}

impl Message {
    fn user(content: String, attachments: Vec<AttachmentInfo>) -> Self {
        Self { role: Role::User, content, tool: None, ok: None, attachments, file: None }
    }
    fn assistant() -> Self {
        Self { role: Role::Assistant, content: String::new(), tool: None, ok: None, attachments: vec![], file: None }
    }
    fn tool_start(name: String) -> Self {
        Self { role: Role::Tool, content: String::new(), tool: Some(name), ok: None, attachments: vec![], file: None }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct AttachmentInfo {
    pub name: String,
    pub path: String,
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct FileResult {
    pub name: String,
    pub path: String,
    pub kind: String,
}

#[derive(Serialize)]
struct ChatPayload {
    messages: Vec<serde_json::Value>,
    model:    String,
    stream:   bool,
}

// ─── Componente ───────────────────────────────────────────────────────────────

#[component]
pub fn ChatPanel() -> impl IntoView {
    let (messages, set_messages)       = signal::<Vec<Message>>(vec![]);
    let (input, set_input)             = signal(String::new());
    let (streaming, set_streaming)     = signal(false);
    let (reasoning_buf, set_reasoning) = signal(String::new());
    let (drag_over, set_drag_over)     = signal(false);
    let (attachments, set_attachments) = signal::<Vec<AttachmentInfo>>(vec![]);

    let scroll_ref = NodeRef::<leptos::html::Div>::new();

    let send = {
        let messages = messages.clone();
        move || {
            let text = input.get().trim().to_string();
            if text.is_empty() || streaming.get() { return; }

            let atts = attachments.get();
            let prefix = build_attachment_prefix(&atts);
            let full_content = if prefix.is_empty() {
                text.clone()
            } else {
                format!("{prefix}{text}")
            };

            set_attachments.set(vec![]);
            set_messages.update(|m| m.push(Message::user(text.clone(), atts)));
            set_input.set(String::new());
            set_streaming.set(true);
            set_reasoning.set(String::new());
            set_messages.update(|m| m.push(Message::assistant()));

            let history: Vec<serde_json::Value> = messages.get().iter().map(|m| {
                serde_json::json!({
                    "role": match m.role { Role::User => "user", Role::Assistant => "assistant", Role::Tool => "tool" },
                    "content": m.content
                })
            }).collect();
            let mut msgs = history;
            msgs.push(serde_json::json!({ "role": "user", "content": full_content }));

            let payload = serde_json::to_string(&serde_json::json!({
                "messages": msgs,
                "model": "",
                "stream": true,
                "tools": *TOOLS
            })).unwrap();

            spawn_sse(payload, set_messages, set_streaming, set_reasoning);
        }
    };

    let send_clone = send.clone();
    let on_keydown = move |e: web_sys::KeyboardEvent| {
        if e.key() == "Enter" && !e.shift_key() {
            e.prevent_default();
            send_clone();
        }
    };

    view! {
        <div
            class="flex flex-col h-full bg-surface relative"
            on:dragover=move |e: web_sys::DragEvent| {
                e.prevent_default();
                set_drag_over.set(true);
            }
            on:dragleave=move |e: web_sys::DragEvent| {
                e.prevent_default();
                if e.related_target().is_none() { set_drag_over.set(false); }
            }
            on:drop=move |e: web_sys::DragEvent| {
                e.prevent_default();
                set_drag_over.set(false);
                if let Some(dt) = e.data_transfer() {
                    if let Some(files) = dt.files() {
                        wasm_bindgen_futures::spawn_local(async move {
                            let new_atts = upload_files(files).await;
                            set_attachments.update(|v| v.extend(new_atts));
                        });
                    }
                }
            }
        >
            // ── Overlay drag ──────────────────────────────────────────────────
            {move || drag_over.get().then(|| view! {
                <div class="absolute inset-0 bg-primary/10 border-2 border-dashed border-primary \
                            rounded-xl flex items-center justify-center z-50 pointer-events-none">
                    <div class="flex flex-col items-center gap-3 text-primary select-none">
                        <span class="material-symbols-outlined text-[52px]">"upload_file"</span>
                        <span class="font-semibold text-sm">"Suelta los archivos aquí"</span>
                    </div>
                </div>
            })}

            // ── Header ────────────────────────────────────────────────────────
            <header class="flex items-center justify-between px-6 py-4 border-b border-outline-var bg-white/60 backdrop-blur-sm shrink-0">
                <div class="flex items-center gap-3">
                    <span class="material-symbols-outlined text-primary text-[22px]">"smart_toy"</span>
                    <span class="font-semibold text-primary text-sm tracking-tight">"LocalAI Assistant"</span>
                </div>
                {move || streaming.get().then(|| view! {
                    <div class="flex items-center gap-2 text-xs text-on-surf-var">
                        <span class="w-1.5 h-1.5 bg-green-500 rounded-full animate-pulse"/>
                        "Procesando..."
                    </div>
                })}
            </header>

            // ── Mensajes ──────────────────────────────────────────────────────
            <div node_ref=scroll_ref class="flex-1 overflow-y-auto px-4 py-6 space-y-4">
                {move || {
                    let msgs = messages.get();
                    if msgs.is_empty() {
                        view! {
                            <div class="flex flex-col items-center justify-center h-full text-center gap-4 text-on-surf-var select-none">
                                <span class="material-symbols-outlined text-[48px] text-outline-var">"chat"</span>
                                <div>
                                    <p class="font-medium text-on-surf">"¿En qué puedo ayudarte?"</p>
                                    <p class="text-sm mt-1">"Puedo leer archivos, crear documentos y ejecutar comandos."</p>
                                    <p class="text-sm mt-1 text-outline-var">"Arrastra archivos aquí para adjuntarlos."</p>
                                </div>
                            </div>
                        }.into_any()
                    } else {
                        msgs.into_iter().map(|m| view! { <MsgBubble msg=m/> })
                            .collect_view()
                            .into_any()
                    }
                }}

                {move || {
                    let r = reasoning_buf.get();
                    (!r.is_empty()).then(|| view! {
                        <div class="text-xs text-on-surf-var italic bg-surf-low rounded-lg px-3 py-2 border-l-2 border-outline-var max-w-2xl">
                            <span class="font-semibold not-italic mr-1">"🧠"</span>
                            {r}
                        </div>
                    })
                }}
            </div>

            // ── Input ─────────────────────────────────────────────────────────
            <div class="shrink-0 border-t border-outline-var bg-white/60 backdrop-blur-sm">
                // Chips de adjuntos pendientes
                {move || {
                    let atts = attachments.get();
                    (!atts.is_empty()).then(|| {
                        let chips = atts.into_iter().enumerate().map(|(i, a)| {
                            let icon = kind_icon(&a.kind).to_string();
                            let name = a.name.clone();
                            view! {
                                <div class="flex items-center gap-1.5 bg-surf-high rounded-full \
                                            px-3 py-1 text-xs text-on-surf border border-outline-var/40">
                                    <span class="material-symbols-outlined text-[14px] text-primary">{icon}</span>
                                    <span class="max-w-[120px] truncate">{name}</span>
                                    <button
                                        class="material-symbols-outlined text-[14px] text-on-surf-var hover:text-on-surf ml-0.5"
                                        on:click=move |_| set_attachments.update(|v| { v.remove(i); })
                                    >
                                        "close"
                                    </button>
                                </div>
                            }
                        }).collect_view();
                        view! {
                            <div class="flex flex-wrap gap-2 px-4 pt-3 max-w-4xl mx-auto">
                                {chips}
                            </div>
                        }
                    })
                }}

                <div class="flex gap-2 items-end px-4 py-4 max-w-4xl mx-auto">
                    // Botón explorador de archivos
                    {
                        let fb_ctx = use_context::<FileBrowserCtx>();
                        fb_ctx.map(|fb| view! {
                            <button
                                title="Explorador de archivos"
                                class="w-11 h-11 rounded-xl flex items-center justify-center shrink-0 \
                                       bg-surf-low border border-outline-var/50 text-on-surf-var \
                                       hover:bg-surf-cont hover:text-on-surf transition-colors \
                                       material-symbols-outlined text-[20px]"
                                on:click=move |_| {
                                    fb.open_with(move |path: String| {
                                        // Detectar tipo y añadir como adjunto
                                        let ext = std::path::Path::new(&path)
                                            .extension()
                                            .and_then(|e| e.to_str())
                                            .unwrap_or("")
                                            .to_ascii_lowercase();
                                        let kind = match ext.as_str() {
                                            "pdf"  => "pdf",
                                            "docx" | "doc" => "docx",
                                            "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" => "image",
                                            "txt" | "md" | "rs" | "py" | "js" | "ts" | "json" |
                                            "toml" | "yaml" | "yml" | "css" | "html" | "sh" => "text",
                                            _ => "other",
                                        }.to_string();
                                        let name = std::path::Path::new(&path)
                                            .file_name()
                                            .and_then(|n| n.to_str())
                                            .unwrap_or("archivo")
                                            .to_string();
                                        set_attachments.update(|v| {
                                            v.push(AttachmentInfo { name, path, kind });
                                        });
                                    });
                                }
                            >
                                "folder_open"
                            </button>
                        })
                    }
                    <textarea
                        class="flex-1 resize-none bg-surf-low rounded-xl px-4 py-3 text-sm \
                               text-on-surf placeholder-on-surf-var border border-outline-var/50 \
                               focus:outline-none focus:ring-1 focus:ring-primary \
                               transition-colors min-h-[44px] max-h-[200px]"
                        placeholder="Escribe un mensaje… (Enter para enviar, Shift+Enter para nueva línea)"
                        rows=1
                        prop:value=move || input.get()
                        on:input=move |e| set_input.set(event_target_value(&e))
                        on:keydown=on_keydown
                    />
                    <button
                        class=move || format!(
                            "w-11 h-11 rounded-xl flex items-center justify-center shrink-0 \
                             transition-all material-symbols-outlined text-[20px] {}",
                            if streaming.get() {
                                "bg-surf-high text-on-surf-var cursor-not-allowed"
                            } else {
                                "bg-primary text-white hover:bg-[#003b65] active:scale-95"
                            }
                        )
                        disabled=move || streaming.get()
                        on:click=move |_| send()
                    >
                        "send"
                    </button>
                </div>
            </div>
        </div>
    }
}

// ─── Burbuja de mensaje ───────────────────────────────────────────────────────

#[component]
fn MsgBubble(msg: Message) -> impl IntoView {
    match msg.role {
        Role::Tool => {
            let tool_name = msg.tool.clone().unwrap_or_default();
            let ok        = msg.ok.unwrap_or(true);
            let file      = msg.file.clone();
            view! {
                <div class="flex flex-col gap-1.5">
                    // Indicador de tool call
                    <div class="flex items-start gap-2 text-xs text-on-surf-var">
                        <span class=move || format!(
                            "material-symbols-outlined text-[16px] mt-0.5 {}",
                            if ok { "text-green-500" } else { "text-red-400" }
                        )>
                            {if ok { "check_circle" } else { "error" }}
                        </span>
                        <span class="font-mono bg-surf-low rounded px-2 py-0.5">{tool_name}</span>
                    </div>
                    // Card de archivo generado
                    {file.map(|f| {
                        let icon = kind_icon(&f.kind).to_string();
                        let display_name  = f.name.clone();
                        let dl_path       = f.path.clone();
                        let dl_name       = f.name.clone();
                        view! {
                            <button
                                class="inline-flex items-center gap-2 bg-surf-low border border-outline-var/50 \
                                       rounded-xl px-4 py-3 text-sm text-primary hover:bg-surf-cont \
                                       transition-colors w-fit max-w-xs shadow-sm cursor-pointer"
                                on:click=move |_| blob_download(&dl_path, &dl_name)
                            >
                                <span class="material-symbols-outlined text-[20px]">{icon}</span>
                                <div class="flex flex-col min-w-0 text-left">
                                    <span class="font-medium truncate">{display_name}</span>
                                    <span class="text-xs text-on-surf-var">"Haz clic para descargar"</span>
                                </div>
                                <span class="material-symbols-outlined text-[16px] text-on-surf-var ml-1 shrink-0">"download"</span>
                            </button>
                        }
                    })}
                </div>
            }.into_any()
        }

        Role::User => {
            let attachments = msg.attachments.clone();
            view! {
                <div class="flex justify-end">
                    <div class="flex flex-col items-end gap-1.5 max-w-[75%]">
                        // Chips de archivos adjuntos (si los hay)
                        {(!attachments.is_empty()).then(|| {
                            let chips = attachments.iter().map(|a| {
                                let icon = kind_icon(&a.kind).to_string();
                                let name = a.name.clone();
                                view! {
                                    <div class="flex items-center gap-1.5 bg-primary/20 rounded-full \
                                                px-3 py-1 text-xs text-primary border border-primary/20">
                                        <span class="material-symbols-outlined text-[12px]">{icon}</span>
                                        <span class="max-w-[140px] truncate">{name}</span>
                                    </div>
                                }
                            }).collect_view();
                            view! {
                                <div class="flex flex-wrap gap-1.5 justify-end">
                                    {chips}
                                </div>
                            }
                        })}
                        // Texto del mensaje
                        <div class="bg-primary text-white rounded-2xl rounded-tr-sm px-4 py-3 text-sm whitespace-pre-wrap leading-relaxed">
                            {msg.content}
                        </div>
                    </div>
                </div>
            }.into_any()
        }

        Role::Assistant => view! {
            <div class="flex justify-start gap-3 max-w-[85%]">
                <div class="w-7 h-7 rounded-full bg-surf-high flex items-center justify-center shrink-0 mt-0.5">
                    <span class="material-symbols-outlined text-[16px] text-primary">"smart_toy"</span>
                </div>
                <div class="bg-white border border-outline-var/30 rounded-2xl rounded-tl-sm px-4 py-3 text-sm text-on-surf leading-relaxed whitespace-pre-wrap shadow-sm">
                    {if msg.content.is_empty() {
                        view! {
                            <span class="flex gap-1 items-center text-on-surf-var">
                                <span class="w-1.5 h-1.5 bg-on-surf-var rounded-full animate-bounce" style="animation-delay:0ms"/>
                                <span class="w-1.5 h-1.5 bg-on-surf-var rounded-full animate-bounce" style="animation-delay:150ms"/>
                                <span class="w-1.5 h-1.5 bg-on-surf-var rounded-full animate-bounce" style="animation-delay:300ms"/>
                            </span>
                        }.into_any()
                    } else {
                        view! { <span>{msg.content.clone()}</span> }.into_any()
                    }}
                </div>
            </div>
        }.into_any(),
    }
}

// ─── Helpers UI ───────────────────────────────────────────────────────────────

fn kind_icon(kind: &str) -> &'static str {
    match kind {
        "pdf"   => "picture_as_pdf",
        "docx"  => "description",
        "image" => "image",
        "text"  => "article",
        _       => "attach_file",
    }
}

/// Descarga un archivo via fetch → Blob → click programático.
/// Funciona en WebView (wry/WKWebView) donde el atributo `download` en <a> no funciona.
fn blob_download(path: &str, filename: &str) {
    // Escapar valores para embeber en JS de forma segura
    let encoded = js_sys::encode_uri_component(path)
        .as_string()
        .unwrap_or_default();
    let safe_name = filename
        .replace('\\', "\\\\")
        .replace('\'', "\\'");

    let code = format!(
        "fetch('/download?path={encoded}') \
         .then(function(r){{return r.blob()}}) \
         .then(function(b){{ \
           var u=URL.createObjectURL(b), \
               a=document.createElement('a'); \
           a.href=u; a.download='{safe_name}'; \
           document.body.appendChild(a); a.click(); \
           document.body.removeChild(a); \
           setTimeout(function(){{URL.revokeObjectURL(u)}},1000); \
         }});"
    );
    js_sys::eval(&code).ok();
}

// ─── Definiciones de tools ────────────────────────────────────────────────────

static TOOLS: std::sync::LazyLock<serde_json::Value> = std::sync::LazyLock::new(|| {
    serde_json::json!([
        {
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Lee el contenido de un archivo de texto o extrae el texto de un PDF.",
                "parameters": {
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "read_docx",
                "description": "Extrae el texto de un archivo DOCX (Word).",
                "parameters": {
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "create_docx",
                "description": "Crea un documento Word (.docx) con el contenido indicado. Soporta markdown: # H1, ## H2, ### H3, - listas, **negrita**.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path":    { "type": "string", "description": "Nombre o ruta del archivo, ej: resumen.docx" },
                        "content": { "type": "string", "description": "Contenido en markdown" }
                    },
                    "required": ["path", "content"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "create_pdf",
                "description": "Crea un documento PDF con el contenido indicado. Soporta markdown básico.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path":    { "type": "string", "description": "Nombre o ruta del archivo, ej: resumen.pdf" },
                        "content": { "type": "string", "description": "Contenido en markdown" }
                    },
                    "required": ["path", "content"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "write_file",
                "description": "Escribe contenido en un archivo de texto.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path":    { "type": "string" },
                        "content": { "type": "string" }
                    },
                    "required": ["path", "content"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "list_files",
                "description": "Lista los archivos de un directorio.",
                "parameters": {
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "run_command",
                "description": "Ejecuta un comando de shell y devuelve la salida.",
                "parameters": {
                    "type": "object",
                    "properties": { "command": { "type": "string" } },
                    "required": ["command"]
                }
            }
        }
    ])
});

// ─── Prefijo de adjuntos ──────────────────────────────────────────────────────

fn build_attachment_prefix(attachments: &[AttachmentInfo]) -> String {
    if attachments.is_empty() { return String::new(); }

    let mut out = String::from("Archivos adjuntos para este mensaje:\n\n");
    for (i, a) in attachments.iter().enumerate() {
        let label = match a.kind.as_str() {
            "text"  => "texto",
            "pdf"   => "PDF",
            "docx"  => "Word",
            "image" => "imagen",
            _       => "archivo",
        };
        out.push_str(&format!("[{}] {} ({})\n", i + 1, a.name, label));
        match a.kind.as_str() {
            "image" => out.push_str(&format!("Ruta: {}\n\n", a.path)),
            "pdf"   => out.push_str(&format!(
                "Ruta: {}\nPara leer: read_file({{\"path\":\"{}\"}})\n\n",
                a.path, a.path
            )),
            "docx"  => out.push_str(&format!(
                "Ruta: {}\nPara leer: read_docx({{\"path\":\"{}\"}})\n\n",
                a.path, a.path
            )),
            _       => out.push_str(&format!(
                "Ruta: {}\nPara leer: read_file({{\"path\":\"{}\"}})\n\n",
                a.path, a.path
            )),
        }
    }
    out.push_str("---\n\n");
    out
}

// ─── Upload de archivos ───────────────────────────────────────────────────────

async fn upload_files(files: web_sys::FileList) -> Vec<AttachmentInfo> {
    let window = match web_sys::window() {
        Some(w) => w,
        None => return vec![],
    };
    let mut results = Vec::new();

    for i in 0..files.length() {
        let file = match files.get(i) { Some(f) => f, None => continue };

        let form_data = match web_sys::FormData::new() { Ok(fd) => fd, Err(_) => continue };
        let file_name = file.name();
        if form_data.append_with_blob_and_filename("file", &*file, &file_name).is_err() { continue; }

        let opts = web_sys::RequestInit::new();
        opts.set_method("POST");
        opts.set_body(form_data.as_ref());

        let req = match web_sys::Request::new_with_str_and_init("/upload", &opts) { Ok(r) => r, Err(_) => continue };

        let resp_val = match wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&req)).await {
            Ok(r) => r, Err(_) => continue,
        };
        let resp = web_sys::Response::from(resp_val);
        let text_promise = match resp.text() { Ok(p) => p, Err(_) => continue };
        let text_val = match wasm_bindgen_futures::JsFuture::from(text_promise).await { Ok(v) => v, Err(_) => continue };
        let text = match text_val.as_string() { Some(s) => s, None => continue };

        if let Ok(infos) = serde_json::from_str::<Vec<AttachmentInfo>>(&text) {
            results.extend(infos);
        }
    }
    results
}

// ─── SSE streaming ────────────────────────────────────────────────────────────

fn spawn_sse(
    payload:        String,
    set_messages:   WriteSignal<Vec<Message>>,
    set_streaming:  WriteSignal<bool>,
    set_reasoning:  WriteSignal<String>,
) {
    wasm_bindgen_futures::spawn_local(async move {
        let window = web_sys::window().unwrap();

        let opts = web_sys::RequestInit::new();
        opts.set_method("POST");
        opts.set_body(&JsValue::from_str(&payload));

        let request = web_sys::Request::new_with_str_and_init("/v1/chat/stream", &opts).unwrap();
        request.headers().set("Content-Type", "application/json").unwrap();

        let resp = match wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&request)).await {
            Ok(r) => web_sys::Response::from(r),
            Err(_) => {
                append_to_last(&set_messages, "\n[Error de conexión]");
                set_streaming.set(false);
                return;
            }
        };

        let body = match resp.body() { Some(b) => b, None => { set_streaming.set(false); return; } };
        let reader = web_sys::ReadableStreamDefaultReader::new(&body).unwrap();
        let decoder = js_sys::eval("new TextDecoder()").unwrap();
        let mut buf = String::new();

        loop {
            let chunk = match wasm_bindgen_futures::JsFuture::from(reader.read()).await {
                Ok(c) => c, Err(_) => break,
            };
            let done = js_sys::Reflect::get(&chunk, &JsValue::from_str("done"))
                .unwrap_or(JsValue::TRUE).as_bool().unwrap_or(true);
            if done { break; }

            let value   = js_sys::Reflect::get(&chunk, &JsValue::from_str("value")).unwrap();
            let decoded = js_sys::Reflect::apply(
                &js_sys::Reflect::get(&decoder, &JsValue::from_str("decode")).unwrap().into(),
                &decoder,
                &js_sys::Array::of1(&value),
            ).unwrap_or_default();
            buf.push_str(&decoded.as_string().unwrap_or_default());

            while let Some(pos) = buf.find("\n\n") {
                let block = buf[..pos].to_string();
                buf = buf[pos + 2..].to_string();

                let mut event_type = String::new();
                let mut data       = String::new();
                for line in block.lines() {
                    if let Some(e) = line.strip_prefix("event: ") { event_type = e.to_string(); }
                    if let Some(d) = line.strip_prefix("data: ")  { data       = d.to_string(); }
                }

                match event_type.as_str() {
                    "token" => {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&data) {
                            if let Some(text) = v["text"].as_str() { append_to_last(&set_messages, text); }
                        }
                    }
                    "reasoning" => {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&data) {
                            if let Some(text) = v["text"].as_str() {
                                set_reasoning.update(|r| r.push_str(text));
                            }
                        }
                    }
                    "tool_start" => {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&data) {
                            let name = v["name"].as_str().unwrap_or("tool").to_string();
                            set_messages.update(|msgs| msgs.push(Message::tool_start(name)));
                        }
                    }
                    "tool_result" => {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&data) {
                            let ok   = v["ok"].as_bool().unwrap_or(true);
                            let file = serde_json::from_value::<FileResult>(v["file"].clone()).ok();

                            set_messages.update(|msgs| {
                                if let Some(last) = msgs.iter_mut().rev()
                                    .find(|m| m.role == Role::Tool && m.ok.is_none())
                                {
                                    last.ok   = Some(ok);
                                    last.file = file;
                                }
                            });
                            set_messages.update(|m| m.push(Message::assistant()));
                        }
                    }
                    "done" | "error" => {
                        set_streaming.set(false);
                        set_reasoning.set(String::new());
                        break;
                    }
                    _ => {}
                }
            }
        }

        set_streaming.set(false);
        set_reasoning.set(String::new());
    });
}

fn append_to_last(set_messages: &WriteSignal<Vec<Message>>, text: &str) {
    let t = text.to_string();
    set_messages.update(|msgs| {
        if let Some(last) = msgs.iter_mut().rev().find(|m| m.role == Role::Assistant) {
            last.content.push_str(&t);
        }
    });
}
