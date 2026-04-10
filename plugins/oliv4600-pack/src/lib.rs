use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{spawn_local, JsFuture};

// ─── View Enum ────────────────────────────────────────────────────────────────
// Each variant maps to a top-level workspace within the OLIV4600 interface.
// The active view is the single source of truth for which screen is rendered.
//
// Navigation mapping (top bar tabs → View):
//   "Editor"        → View::Editor      (3-col workspace: source | controls | output)
//   "Analysis"      → View::Analysis    (bento grid: readability, NER, sentiment, timeline)
//   "Chat"          → View::Chat        (50/50 split: document viewer + conversation panel)
//   "Verifiability" → View::Pipeline    (horizontal derivation chain / production pipeline)
//   "Audit"         → View::Audit       (tamper-evident operation log — Phase 8 placeholder)

#[derive(Clone, Copy, PartialEq)]
enum View {
    Dashboard,
    Editor,
    Analysis,
    Chat,
    Pipeline,
    Audit,
}

// ─── Static sample data ───────────────────────────────────────────────────────
// TODO: Replace with live data fetched from /api/projects (GET) which reads
// from ~/.local-ai/workspace/. Each entry represents a root document with
// its processing history stored in SQLite (TRA-001, TRA-002).

#[derive(Clone)]
struct Transformation {
    name:  &'static str,
    kind:  &'static str,
    badge: &'static str,
    ts:    &'static str,
}

const TRANSFORMATIONS: &[Transformation] = &[
    Transformation {
        name:  "Q4_Budget_Review_V2.pdf",
        kind:  "Executive Summary",
        badge: "bg-[#003b65] text-[#66a6ea]",
        ts:    "Apr 08, 2026 — 14:32",
    },
    Transformation {
        name:  "Annual_Sustainability_Report.docx",
        kind:  "Press Release Draft",
        badge: "bg-[#622700] text-[#fa813a]",
        ts:    "Apr 07, 2026 — 09:15",
    },
    Transformation {
        name:  "Strategic_Vision_2027.odt",
        kind:  "LinkedIn Optimization",
        badge: "bg-surf-highest text-on-surf-var",
        ts:    "Apr 06, 2026 — 18:44",
    },
];

// ─── Document Context ─────────────────────────────────────────────────────────
// Compartido vía Leptos context (provide_context / use_context).
// Creado en App, leído/escrito por todas las vistas.

#[derive(Clone, Copy)]
struct DocumentCtx {
    /// Texto completo del documento cargado (ING-001..ING-003)
    text:         RwSignal<String>,
    /// Nombre del archivo cargado
    filename:     RwSignal<String>,
    /// Número de palabras
    word_count:   RwSignal<u32>,
    /// True mientras hay procesamiento en curso
    processing:   RwSignal<bool>,
    /// Texto generado (streaming token por token)
    output:       RwSignal<String>,
    /// Etiqueta de la última acción ejecutada
    output_label: RwSignal<String>,
}

impl DocumentCtx {
    fn new() -> Self {
        Self {
            text:         RwSignal::new(String::new()),
            filename:     RwSignal::new(String::new()),
            word_count:   RwSignal::new(0),
            processing:   RwSignal::new(false),
            output:       RwSignal::new(String::new()),
            output_label: RwSignal::new(String::new()),
        }
    }
}

// ─── Helper: upload + extracción de texto (ING-001, ING-002) ─────────────────
// POST /upload (multipart) → { path, name }
// POST /api/extract → { text, word_count, filename }
// En éxito: actualiza ctx y navega a View::Editor.

fn upload_and_load(file: web_sys::File, ctx: DocumentCtx, set_active_view: WriteSignal<View>) {
    spawn_local(async move {
        ctx.processing.set(true);

        // Step 1: upload
        let form_data = web_sys::FormData::new().unwrap();
        form_data.append_with_blob("file", &file).unwrap();
        let opts = web_sys::RequestInit::new();
        opts.set_method("POST");
        opts.set_body(&form_data.into());
        let request = web_sys::Request::new_with_str_and_init("/upload", &opts).unwrap();
        let window  = web_sys::window().unwrap();
        let resp: web_sys::Response = match JsFuture::from(window.fetch_with_request(&request)).await {
            Ok(r)  => r.unchecked_into(),
            Err(_) => { ctx.processing.set(false); return; }
        };
        let json_str = match JsFuture::from(resp.text().unwrap()).await {
            Ok(t) => t.as_string().unwrap_or_default(),
            Err(_) => { ctx.processing.set(false); return; }
        };
        let uploads: Vec<serde_json::Value> = serde_json::from_str(&json_str).unwrap_or_default();
        let path = match uploads.first().and_then(|u| u["path"].as_str()) {
            Some(p) => p.to_string(),
            None    => { ctx.processing.set(false); return; }
        };

        // Step 2: extract text
        let extract_body = serde_json::json!({"path": path}).to_string();
        let headers = web_sys::Headers::new().unwrap();
        headers.set("Content-Type", "application/json").unwrap();
        let opts2 = web_sys::RequestInit::new();
        opts2.set_method("POST");
        opts2.set_body(&wasm_bindgen::JsValue::from_str(&extract_body));
        opts2.set_headers(&wasm_bindgen::JsValue::from(headers));
        let req2 = web_sys::Request::new_with_str_and_init("/api/extract", &opts2).unwrap();
        let resp2: web_sys::Response = match JsFuture::from(window.fetch_with_request(&req2)).await {
            Ok(r)  => r.unchecked_into(),
            Err(_) => { ctx.processing.set(false); return; }
        };
        let json2 = match JsFuture::from(resp2.text().unwrap()).await {
            Ok(t) => t.as_string().unwrap_or_default(),
            Err(_) => { ctx.processing.set(false); return; }
        };
        if let Ok(ex) = serde_json::from_str::<serde_json::Value>(&json2) {
            if let Some(t) = ex["text"].as_str()     { ctx.text.set(t.to_string()); }
            if let Some(f) = ex["filename"].as_str() { ctx.filename.set(f.to_string()); }
            if let Some(w) = ex["word_count"].as_u64() { ctx.word_count.set(w as u32); }
            set_active_view.set(View::Editor);
        }
        ctx.processing.set(false);
    });
}

// ─── Helper: llamada al motor de transformación (SRS §8.3) ───────────────────
// POST /api/transform con SSE streaming.
// Los tokens van acumulándose en ctx.output en tiempo real.

fn run_transform(
    ctx: DocumentCtx, action: String, length_words: u32, tone: u32,
    audience: String, language: String,
) {
    spawn_local(async move {
        ctx.processing.set(true);
        ctx.output.set(String::new());
        ctx.output_label.set(action_label(&action).to_string());
        let body = serde_json::json!({
            "text": ctx.text.get_untracked(), "action": action,
            "length_words": length_words, "tone": tone.to_string(),
            "audience": audience, "language": language,
        }).to_string();
        let headers = web_sys::Headers::new().unwrap();
        headers.set("Content-Type", "application/json").unwrap();
        let opts = web_sys::RequestInit::new();
        opts.set_method("POST");
        opts.set_body(&wasm_bindgen::JsValue::from_str(&body));
        opts.set_headers(&wasm_bindgen::JsValue::from(headers));
        let req = web_sys::Request::new_with_str_and_init("/api/transform", &opts).unwrap();
        let window = web_sys::window().unwrap();
        let resp: web_sys::Response = match JsFuture::from(window.fetch_with_request(&req)).await {
            Ok(r) => r.unchecked_into(),
            Err(_) => { ctx.processing.set(false); return; }
        };
        read_sse_stream(resp,
            move |t| ctx.output.update(|s| s.push_str(&t)),
            move || ctx.processing.set(false),
        ).await;
    });
}

// ─── Helper: chat sobre documento (CHA-001) ───────────────────────────────────
// POST /v1/chat/stream con contexto del documento como system prompt.

fn run_chat(ctx: DocumentCtx, messages: RwSignal<Vec<(String, String)>>, user_msg: String) {
    spawn_local(async move {
        ctx.processing.set(true);
        messages.update(|v| v.push(("user".to_string(), user_msg.clone())));
        let doc = ctx.text.get_untracked();
        let sys = if doc.is_empty() {
            "Eres OLIV4600, asistente de procesamiento documental. No hay documento cargado.".to_string()
        } else {
            format!(
                "Eres OLIV4600. El usuario ha cargado este documento:\n\n---\n{doc}\n---\n\n\
                 Responde preguntas sobre él. Cita el texto fuente cuando sea relevante."
            )
        };
        let msg_array: Vec<serde_json::Value> = std::iter::once(
            serde_json::json!({"role":"system","content":sys})
        ).chain(
            messages.get_untracked().iter().map(|(r, c)| serde_json::json!({"role":r,"content":c}))
        ).collect();
        let ai_idx = messages.get_untracked().len();
        messages.update(|v| v.push(("assistant".to_string(), String::new())));
        let body = serde_json::json!({"messages": msg_array, "stream": true}).to_string();
        let headers = web_sys::Headers::new().unwrap();
        headers.set("Content-Type", "application/json").unwrap();
        let opts = web_sys::RequestInit::new();
        opts.set_method("POST");
        opts.set_body(&wasm_bindgen::JsValue::from_str(&body));
        opts.set_headers(&wasm_bindgen::JsValue::from(headers));
        let req = web_sys::Request::new_with_str_and_init("/v1/chat/stream", &opts).unwrap();
        let window = web_sys::window().unwrap();
        let resp: web_sys::Response = match JsFuture::from(window.fetch_with_request(&req)).await {
            Ok(r) => r.unchecked_into(),
            Err(_) => { ctx.processing.set(false); return; }
        };
        read_sse_stream(resp,
            move |t| messages.update(|v| { if let Some(e) = v.get_mut(ai_idx) { e.1.push_str(&t); } }),
            move || ctx.processing.set(false),
        ).await;
    });
}

// ─── Helper: leer stream SSE ──────────────────────────────────────────────────
// Lee un ReadableStream de texto/event-stream, parsea eventos y llama a on_token
// por cada fragmento `event: token  data: {"text":"..."}` recibido.

async fn read_sse_stream<F: Fn(String), D: Fn()>(resp: web_sys::Response, on_token: F, on_done: D) {
    let reader: web_sys::ReadableStreamDefaultReader = match resp.body() {
        Some(b) => b.get_reader().unchecked_into(),
        None    => { on_done(); return; }
    };
    let mut buf = String::new();
    loop {
        let chunk = match JsFuture::from(reader.read()).await { Ok(c) => c, Err(_) => break };
        if js_sys::Reflect::get(&chunk, &wasm_bindgen::JsValue::from_str("done"))
            .ok().and_then(|v| v.as_bool()).unwrap_or(true) { break; }
        let value = match js_sys::Reflect::get(&chunk, &wasm_bindgen::JsValue::from_str("value")) {
            Ok(v) => v, Err(_) => break,
        };
        buf.push_str(&String::from_utf8_lossy(&js_sys::Uint8Array::new(&value).to_vec()));
        while let Some(idx) = buf.find("\n\n") {
            let msg = buf[..idx].to_string();
            buf = buf[idx + 2..].to_string();
            for line in msg.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    if let Ok(ev) = serde_json::from_str::<serde_json::Value>(data) {
                        if let Some(t) = ev["text"].as_str() { if !t.is_empty() { on_token(t.to_string()); } }
                    }
                }
            }
        }
    }
    on_done();
}

// ─── Helper: copiar al portapapeles (EXO-001) ─────────────────────────────────

fn copy_to_clipboard(text: String) {
    spawn_local(async move {
        if let Some(w) = web_sys::window() {
            if let Some(cb) = w.navigator().clipboard() {
                let _ = JsFuture::from(cb.write_text(&text)).await;
            }
        }
    });
}

// ─── Helper: etiqueta legible de una acción ───────────────────────────────────

fn action_label(action: &str) -> &'static str {
    match action {
        "executive_summary"     => "Resumen Ejecutivo",
        "technical_summary"     => "Resumen Técnico",
        "divulgative_summary"   => "Resumen Divulgativo",
        "bullet_summary"        => "Puntos Clave",
        "chronological_summary" => "Resumen Cronológico",
        "conclusions_summary"   => "Conclusiones y Recomendaciones",
        "briefing_2min"         => "Briefing 2 min",
        "press_release"         => "Nota de Prensa",
        "headlines"             => "Titulares",
        "linkedin_post"         => "Post LinkedIn",
        "twitter_thread"        => "Hilo Twitter/X",
        "blog_article"          => "Artículo de Blog",
        "instagram_post"        => "Post Instagram",
        "email_newsletter"      => "Email / Newsletter",
        "speech"                => "Discurso",
        "faqs"                  => "FAQs",
        "one_pager"             => "One-Pager / Ficha Resumen",
        "key_quotes"            => "Citas Textuales",
        "official_report"       => "Informe Oficial",
        "meeting_minutes"       => "Acta de Reunión",
        "administrative_resolution" => "Resolución Administrativa",
        "internal_memo"         => "Memorando Interno",
        "allegations_response"  => "Alegaciones / Negociación",
        "extract_commitments"   => "Compromisos Verificables",
        "rewrite_formal"        => "Reescritura Formal",
        "rewrite_shorter"       => "Reescritura Concisa",
        "rewrite_persuasive"    => "Reescritura Persuasiva",
        "rewrite_clearer"       => "Reescritura Clara",
        "detect_redundancies"   => "Detectar Redundancias",
        "translate_language"    => "Traducción",
        "sentiment_analysis"    => "Análisis de Sentimiento",
        "grammar_check"         => "Corrección Gramatical",
        "simplify"              => "Simplificar (Lenguaje Claro)",
        "detect_inconsistencies"=> "Detectar Inconsistencias",
        "reformulate_paragraph" => "Reformular Párrafo",
        "detect_ambiguities"    => "Detectar Ambigüedades",
        "improve_suggestions"   => "Sugerencias de Mejora",
        "readability_analysis"  => "Análisis de Legibilidad",
        "detect_evasive_language"=> "Lenguaje Evasivo",
        "semantic_versioning"   => "Versionado Semántico",
        "merge_documents"       => "Fusión de Documentos",
        "semantic_diff"         => "Diferencial Semántico",
        "document_intersection" => "Intersección Documental",
        "detect_contradictions" => "Detectar Contradicciones",
        "versions_compare"      => "Comparar Versiones",
        "inverse_questions"     => "Preguntas Inversas (Editor Jefe)",
        "press_release_check"   => "Verificar Nota de Prensa",
        "validation_questions"  => "Checklist de Validación",
        "ner_extraction"        => "Extracción de Entidades (NER)",
        "keywords_extraction"   => "Palabras Clave y Categorías",
        "event_timeline"        => "Línea Temporal",
        "impact_analysis"       => "Análisis de Impacto",
        "verifiability_check"   => "Verificabilidad y Soporte",
        "evidence_gaps"         => "Huecos de Evidencia",
        "traceability_map"      => "Mapa de Trazabilidad",
        "anonymize"             => "Anonimización / Expurgo",
        "preflight_check"       => "Preflight Documental",
        "public_version"        => "Versión Pública",
        "rgpd_check"            => "Verificación RGPD/LOPDGDD",
        "style_linting"         => "Linting Documental",
        "reader_simulation"     => "Simulador de Lector",
        "generate_from_form"    => "Generar desde Formulario",
        "generate_file_package" => "Paquete de Expediente",
        "crisis_press_questions"=> "Simulacro Comparecencia",
        "crisis_communication"  => "Kit de Crisis Reputacional",
        "argumentario"          => "Argumentario",
        "difficult_questions_simulator" => "Simulador Preguntas Difíciles",
        _                       => "Transformación",
    }
}

// ─── Helper: etiqueta de tono ─────────────────────────────────────────────────

fn tone_label(t: u32) -> &'static str {
    match t {
        1 => "Coloquial",
        2 => "Periodístico",
        3 => "Divulgativo",
        4 => "Técnico",
        5 => "Formal",
        _ => "Técnico",
    }
}

// ─── WASM entry point ─────────────────────────────────────────────────────────

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn main() {
    leptos::mount::mount_to_body(App);
}

// ─── Root component ───────────────────────────────────────────────────────────
// Owns all top-level reactive state and composes the layout shell.
//
// TODO: Add a `loaded_document: RwSignal<Option<Document>>` here so that every
// child view can read the currently loaded document without prop-drilling.
// Document should carry: raw_text, filename, language (ING-006), word_count.
//
// TODO: Add a `processing: RwSignal<bool>` for global loading overlay during
// LLM inference (PER-001 target: <15s for simple summaries).

#[component]
fn App() -> impl IntoView {
    let (active_nav, set_active_nav) = signal("Projects");
    let (active_view, set_active_view) = signal(View::Dashboard);
    let (drag_over, set_drag_over)     = signal(false);

    // Contexto de documento compartido por todas las vistas
    let doc_ctx = DocumentCtx::new();
    provide_context(doc_ctx);

    view! {
        <div class="flex h-screen overflow-hidden bg-surface font-sans">
            <Sidebar active_nav set_active_nav set_active_view/>
            <div class="flex-1 flex flex-col overflow-hidden">
                <TopBar active_view set_active_view/>
                // Chat y Editor usan layout flex con scroll interno;
                // el resto hace scroll en este contenedor.
                <main class=move || match active_view.get() {
                    View::Chat | View::Editor => "flex-1 overflow-hidden",
                    _                         => "flex-1 overflow-y-auto",
                }>
                    // Overlay de procesamiento global
                    {move || doc_ctx.processing.get().then(|| view! {
                        <div class="fixed inset-0 z-[100] flex items-center justify-center bg-black/30 backdrop-blur-sm pointer-events-none">
                            <div class="bg-primary text-[#66a6ea] px-6 py-3 rounded-lg flex items-center gap-3 shadow-2xl">
                                <div class="w-2 h-2 rounded-full bg-[#66a6ea] animate-pulse"></div>
                                <span class="text-xs font-black uppercase tracking-widest">
                                    "Procesando..."
                                </span>
                            </div>
                        </div>
                    })}
                    {move || match active_view.get() {
                        View::Dashboard => view! { <DashboardView set_active_view drag_over set_drag_over/> }.into_any(),
                        View::Editor    => view! { <EditorView set_active_view/> }.into_any(),
                        View::Analysis  => view! { <AnalysisView/> }.into_any(),
                        View::Chat      => view! { <ChatView/> }.into_any(),
                        View::Pipeline  => view! { <PipelineView/> }.into_any(),
                        View::Audit     => view! { <AuditView/> }.into_any(),
                    }}
                </main>
            </div>
        </div>
    }
}

// ─── Sidebar ──────────────────────────────────────────────────────────────────
// Fixed left rail at #002542 (institutional navy). Intentionally minimal — this
// is a tool, not a consumer app.
//
// TODO (Module 12 — Templates & Context): Populate the Templates section
// dynamically from /api/templates, reading ~/.local-ai/templates/.
// Each template carries a document type schema used by the Guided Form
// Constructor (Module 16 — GUI-001..GUI-004).
//
// TODO (Projects): Load recent projects from /api/projects sorted by
// last-modified. Each project is a directory under ~/.local-ai/workspace/
// containing the source document + all derived outputs + audit log.

#[component]
fn Sidebar(
    active_nav:     ReadSignal<&'static str>,
    set_active_nav: WriteSignal<&'static str>,
    set_active_view: WriteSignal<View>,
) -> impl IntoView {
    view! {
        <aside class="bg-[#002542] w-[280px] h-full flex flex-col py-8 px-4 shrink-0">
            // Clicking the brand mark returns to the Dashboard home screen.
            <div
                class="mb-10 px-2 cursor-pointer select-none"
                on:click=move |_| set_active_view.set(View::Dashboard)
            >
                <h1 class="text-2xl font-black tracking-tighter text-white">"OLIV4600"</h1>
                <p class="uppercase text-[11px] font-bold text-[#66a6ea] tracking-tight">
                    "Sovereign Intelligence"
                </p>
            </div>

            <nav class="flex-1 space-y-6 overflow-y-auto">
                <NavSection label="Main">
                    // TODO (Projects): navigating here should show the project browser —
                    // a grid of document cards with status badges (draft / processed / exported).
                    <NavItem icon="folder_open"  label="Projects"  active=active_nav set_active=set_active_nav/>
                    // TODO (Module 12 — Templates): show the template library where users can
                    // browse, preview, and apply organizational document templates (PLT-001..PLT-004).
                    <NavItem icon="edit_note"    label="Templates" active=active_nav set_active=set_active_nav/>
                    // TODO (Library): shows all exported documents from ~/.local-ai/workspace/
                    // with full-text search (EXT-003 keyword index), filter by type, sort by date.
                    <NavItem icon="inventory_2"  label="Library"   active=active_nav set_active=set_active_nav/>
                    // TODO (AI Engine): configuration panel for the local LLM instance —
                    // model selection, inference parameters (temperature, context window),
                    // Ollama endpoint health, and memory/disk usage stats.
                    <NavItem icon="memory"       label="AI Engine" active=active_nav set_active=set_active_nav/>
                </NavSection>

                <NavSection label="Recent Projects">
                    <div class="space-y-0.5 px-2">
                        // TODO: Each recent item should be clickable to re-open that project,
                        // loading its source document into the global document context and
                        // navigating to View::Editor. Truncate label to 28 chars with ellipsis.
                        <RecentItem label="Q4 Fiscal Policy Draft"/>
                        <RecentItem label="Institutional Audit Log"/>
                        <RecentItem label="Press Release: Sovereign..."/>
                    </div>
                </NavSection>

                <NavSection label="Templates">
                    <div class="space-y-0.5 px-2">
                        // TODO (Module 16 — GUI-003): clicking a template opens the Guided Form
                        // Constructor pre-filled with that document type's field schema, so the
                        // user can generate a full output package without an existing source file.
                        <TemplateItem label="Press Release"/>
                        <TemplateItem label="Internal Report"/>
                        <TemplateItem label="Meeting Minutes"/>
                    </div>
                </NavSection>
            </nav>

            <div class="mt-auto pt-6 space-y-4">
                // TODO: "New Document" should open a modal that lets the user choose:
                //   a) Upload file (PDF/DOCX/TXT/ODT/HTML — ING-001, ING-002)
                //      → POST /api/upload, extract text server-side (pdfextract / docx-rs)
                //   b) Inline rich-text editor (ING-003) — a Notion-lite editor embedded in the
                //      left panel of EditorView, supporting markdown shortcuts
                //   c) Guided Form Constructor (Module 16 — GUI-001) — structured fields
                //      whose values are injected into the LLM prompt as a virtual source doc
                // On completion, load document into global context → navigate to View::Editor.
                <button class="w-full bg-[#003b65] text-[#66a6ea] py-3 px-4 rounded-lg font-bold text-sm flex items-center justify-center gap-2 hover:bg-[#004d80] transition-colors active:scale-[0.98]">
                    <span class="material-symbols-outlined text-[18px]">"add"</span>
                    "New Document"
                </button>

                // ── Status Ribbon ─────────────────────────────────────────────
                // Hardware-label style indicators. Styled per Design System §5 "Status Ribbon":
                // 1.5px stroke feel, embossed on the UI, no decorative borders.
                //
                // TODO (SEC-001): Drive "System Offline" from a backend flag that verifies
                // no outbound HTTP connections are active. If the app is ever mis-configured
                // to call an external LLM endpoint, this badge should turn amber/red.
                //
                // TODO: "Qwen 3.5 Active" should be populated from GET /api/settings →
                // { llm_model, llm_endpoint }. Add a heartbeat ping (HEAD /api/chat) to
                // confirm the model is actually loaded; show a spinner if warming up.
                <div class="flex flex-col gap-2 border-t border-slate-700/50 pt-4">
                    <StatusRow icon="shield" color="text-[#66a6ea]" label="SYSTEM OFFLINE"/>
                    <StatusRowPulse icon="bolt" label="Qwen 3.5 Active"/>
                </div>
            </div>
        </aside>
    }
}

// ─── Sidebar sub-components ───────────────────────────────────────────────────

#[component]
fn NavSection(label: &'static str, children: Children) -> impl IntoView {
    view! {
        <div>
            <span class="px-2 mb-2 block uppercase text-[11px] font-bold text-slate-400 tracking-tight">
                {label}
            </span>
            {children()}
        </div>
    }
}

#[component]
fn NavItem(
    icon:       &'static str,
    label:      &'static str,
    active:     ReadSignal<&'static str>,
    set_active: WriteSignal<&'static str>,
) -> impl IntoView {
    view! {
        <button
            class=move || format!(
                "w-full flex items-center gap-3 px-3 py-2.5 rounded-lg text-left text-sm font-semibold transition-colors {}",
                if active.get() == label {
                    "bg-[#001b30] text-[#66a6ea] border-r-2 border-[#66a6ea]"
                } else {
                    "text-slate-400 hover:text-white hover:bg-[#00335c]"
                }
            )
            on:click=move |_| set_active.set(label)
        >
            <span class="material-symbols-outlined text-[20px]">{icon}</span>
            {label}
        </button>
    }
}

#[component]
fn RecentItem(label: &'static str) -> impl IntoView {
    view! {
        <div class="text-[13px] text-slate-300 py-1 cursor-pointer hover:text-white truncate">
            {label}
        </div>
    }
}

#[component]
fn TemplateItem(label: &'static str) -> impl IntoView {
    view! {
        <div class="text-[13px] text-slate-300 py-1 cursor-pointer hover:text-white flex items-center gap-2">
            <span class="w-1.5 h-1.5 rounded-full bg-slate-500 shrink-0"></span>
            {label}
        </div>
    }
}

#[component]
fn StatusRow(icon: &'static str, color: &'static str, label: &'static str) -> impl IntoView {
    view! {
        <div class="flex items-center gap-3 px-2 py-1.5">
            <span class=format!("material-symbols-outlined text-[18px] {color}")>{icon}</span>
            <span class=format!("uppercase text-[10px] font-bold {color} tracking-widest")>{label}</span>
        </div>
    }
}

#[component]
fn StatusRowPulse(icon: &'static str, label: &'static str) -> impl IntoView {
    view! {
        <div class="flex items-center gap-3 px-2 py-1.5">
            <div class="relative">
                <span class="material-symbols-outlined text-[18px] text-green-400">{icon}</span>
                <span class="absolute -top-1 -right-1 w-2 h-2 bg-green-400 rounded-full animate-pulse"></span>
            </div>
            <span class="uppercase text-[10px] font-bold text-white tracking-widest">{label}</span>
        </div>
    }
}

// ─── Top Bar ──────────────────────────────────────────────────────────────────
// Shared header present across all views. Tab navigation changes the active view.
// The active tab is visually marked with the action color (#C45911) underline.
//
// TODO: The search bar should query the document archive using a full-text index
// (SQLite FTS5) exposed at GET /api/search?q=... — results should appear in a
// floating dropdown with document name, excerpt, and last-modified date.
//
// TODO: The "Execute Process" button should be context-sensitive:
//   - On Dashboard: disabled (greyed) until a document is loaded
//   - On Editor: triggers the selected transformation with current parameters
//   - On Analysis: re-runs the full forensic analysis suite on the loaded document
//   - On Pipeline: regenerates all derived outputs from the source

#[component]
fn TopBar(
    active_view:     ReadSignal<View>,
    set_active_view: WriteSignal<View>,
) -> impl IntoView {
    view! {
        <header class="bg-white/85 backdrop-blur-xl flex justify-between items-center h-16 px-8 shrink-0 border-b border-slate-200/50 shadow-sm sticky top-0 z-40">
            <div class="flex items-center gap-8">
                <div class="relative flex items-center">
                    <span class="material-symbols-outlined absolute left-3 text-slate-400 text-[20px]">"search"</span>
                    // TODO: wire to GET /api/search?q={value} with 300ms debounce
                    <input
                        class="pl-10 pr-4 py-2 bg-surf-low border-none rounded-lg text-sm w-64 focus:outline-none focus:ring-1 focus:ring-primary"
                        placeholder="Search archive..."
                        type="text"
                    />
                </div>
                <nav class="flex gap-6">
                    <TopTab label="Editor"        view=View::Editor        active=active_view set_active=set_active_view/>
                    <TopTab label="Analysis"      view=View::Analysis      active=active_view set_active=set_active_view/>
                    <TopTab label="Chat"          view=View::Chat          active=active_view set_active=set_active_view/>
                    <TopTab label="Verifiability" view=View::Pipeline      active=active_view set_active=set_active_view/>
                    <TopTab label="Audit"         view=View::Audit         active=active_view set_active=set_active_view/>
                </nav>
            </div>
            <div class="flex items-center gap-4">
                <div class="flex gap-2 mr-4 border-r border-slate-200 pr-4">
                    // TODO (SEC-005): history_edu icon → opens the Audit Log (View::Audit),
                    // showing the last N operations with timestamps and SHA-256 output hashes.
                    <IconBtn icon="shield"/>
                    <IconBtn icon="history_edu"/>
                    // TODO: settings icon → opens the Settings panel (config_panel equivalent)
                    // for LLM endpoint, model, org identity (PLT-002), and export preferences.
                    <IconBtn icon="settings"/>
                </div>
                // TODO: context-sensitive action — see comment above TopBar component
                <button class="px-4 py-2 bg-primary text-white rounded-lg font-bold text-sm hover:bg-[#003b65] transition-colors active:opacity-80">
                    "Execute Process"
                </button>
            </div>
        </header>
    }
}

#[component]
fn TopTab(
    label:      &'static str,
    view:       View,
    active:     ReadSignal<View>,
    set_active: WriteSignal<View>,
) -> impl IntoView {
    view! {
        <button
            class=move || format!(
                "font-serif italic text-lg transition-all {}",
                if active.get() == view {
                    "text-action border-b-2 border-action pb-1"
                } else {
                    "text-slate-500 hover:text-primary"
                }
            )
            on:click=move |_| set_active.set(view)
        >
            {label}
        </button>
    }
}

#[component]
fn IconBtn(icon: &'static str) -> impl IntoView {
    view! {
        <button class="p-2 text-slate-500 hover:bg-surf-cont rounded-lg transition-colors">
            <span class="material-symbols-outlined">{icon}</span>
        </button>
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// VIEW: DASHBOARD (Home)
// ═══════════════════════════════════════════════════════════════════════════════
// Landing screen shown before any document is loaded.
// Provides the primary document ingestion entry point (drag-drop + file type buttons)
// and quick-action shortcuts to the most common transformation types.
//
// Design: asymmetric bento grid with a hero upload zone, 5-col action grid,
// and a recent-transformations table. Floating "Sovereign Protocol Active" pill
// at the bottom of the upload zone acts as the Status Ribbon per DESIGN.md §5.

#[component]
fn DashboardView(
    set_active_view: WriteSignal<View>,
    drag_over:       ReadSignal<bool>,
    set_drag_over:   WriteSignal<bool>,
) -> impl IntoView {
    let ctx           = use_context::<DocumentCtx>().expect("DocumentCtx");
    let file_input_ref = NodeRef::<leptos::html::Input>::new();

    view! {
        <section class="p-8 max-w-7xl mx-auto space-y-8">

            // Input de archivo oculto — lo activan los botones y el drag-drop
            // (ING-001, ING-002)
            <input
                type="file"
                accept=".pdf,.docx,.txt,.odt,.html,.htm,.md,.csv"
                class="hidden"
                node_ref=file_input_ref
                on:change=move |_| {
                    if let Some(input) = file_input_ref.get() {
                        if let Some(files) = input.files() {
                            if let Some(file) = files.get(0) {
                                upload_and_load(file, ctx, set_active_view);
                            }
                        }
                    }
                }
            />

            // ── Hero: Drag & Drop Zone (ING-001, ING-002) ─────────────────────
            <div class="relative">
                <div
                    class=move || format!(
                        "bg-white border-2 border-dashed rounded-xl p-16 text-center flex flex-col items-center justify-center transition-all group {}",
                        if drag_over.get() { "border-primary bg-[#002542]/5" }
                        else               { "border-outline-var hover:border-[#66a6ea]" }
                    )
                    on:dragover=move |e| { e.prevent_default(); set_drag_over.set(true); }
                    on:dragleave=move |_| set_drag_over.set(false)
                    on:drop=move |e: leptos::ev::DragEvent| {
                        e.prevent_default();
                        set_drag_over.set(false);
                        if let Some(dt) = e.data_transfer() {
                            if let Some(files) = dt.files() {
                                if let Some(file) = files.get(0) {
                                    upload_and_load(file, ctx, set_active_view);
                                }
                            }
                        }
                    }
                >
                    <div class="w-16 h-16 bg-surf-low rounded-full flex items-center justify-center mb-6 text-primary group-hover:scale-110 transition-transform">
                        <span class="material-symbols-outlined text-[32px]">"upload_file"</span>
                    </div>
                    <h2 class="font-serif italic text-3xl text-primary mb-2">
                        "Arrastra un documento o empieza a escribir"
                    </h2>
                    <p class="text-on-surf-var text-sm mb-8">
                        "Procesamiento 100% local. Tus datos nunca salen de este equipo."
                    </p>
                    <div class="flex flex-wrap justify-center gap-3">
                        // ING-001: PDF
                        <FileBtn icon="picture_as_pdf" label="PDF"
                            on_click=move || { if let Some(i) = file_input_ref.get() { i.click(); } }
                        />
                        // ING-002: DOCX / TXT / ODT
                        <FileBtn icon="description" label="DOCX"
                            on_click=move || { if let Some(i) = file_input_ref.get() { i.click(); } }
                        />
                        <FileBtn icon="article" label="TXT"
                            on_click=move || { if let Some(i) = file_input_ref.get() { i.click(); } }
                        />
                        // ING-003: editor en blanco
                        <button
                            class="flex items-center gap-2 px-5 py-2.5 bg-primary text-white rounded-lg font-bold text-xs uppercase tracking-wider shadow-lg shadow-primary/20 hover:-translate-y-0.5 transition-all"
                            on:click=move |_| {
                                ctx.text.set(String::new());
                                ctx.filename.set("Nuevo documento".to_string());
                                ctx.word_count.set(0);
                                set_active_view.set(View::Editor);
                            }
                        >
                            <span class="material-symbols-outlined text-[18px]">"add_circle"</span>
                            "Nuevo Documento"
                        </button>
                    </div>
                </div>
                // Status Ribbon — always visible below the drop zone
                <div class="absolute -bottom-3 left-1/2 -translate-x-1/2 px-4 py-1.5 bg-primary text-[#66a6ea] border border-[#66a6ea]/30 rounded-full flex items-center gap-2 shadow-xl whitespace-nowrap">
                    <div class="w-1.5 h-1.5 bg-[#66a6ea] rounded-full animate-pulse"></div>
                    <span class="text-[10px] font-black uppercase tracking-widest">
                        "Sovereign Protocol Active"
                    </span>
                </div>
            </div>

            // ── Quick Actions Grid (asymmetric 5-col) ──────────────────────────
            // These cards are shortcuts into specific transformation modules.
            // Clicking any card should: check if a document is loaded → if not,
            // prompt the user to upload first → if yes, navigate to EditorView
            // with the corresponding action pre-selected in the control panel.
            <div class="grid grid-cols-1 md:grid-cols-5 gap-6">
                // Feature card (2-col wide) — primary CTA for advanced processing
                // TODO: "Initialize Engine" should run a system check:
                //   1. Ping Ollama endpoint → verify model is loaded
                //   2. Check available VRAM/RAM for the selected model
                //   3. Display a readiness checklist in a modal
                <div class="md:col-span-2 bg-primary text-white p-8 rounded-xl flex flex-col justify-between min-h-[240px] relative overflow-hidden group">
                    <div class="relative z-10">
                        <span class="material-symbols-outlined text-[40px] text-[#66a6ea] mb-4 block">"auto_awesome"</span>
                        <h3 class="font-serif italic text-2xl mb-2">"Advanced Transformation"</h3>
                        <p class="text-slate-400 text-sm leading-relaxed max-w-xs">
                            "Convert raw data into professional intelligence reports using the Qwen 3.5 engine."
                        </p>
                    </div>
                    <button class="relative z-10 mt-6 self-start px-4 py-2 bg-[#66a6ea] text-primary rounded-lg font-bold text-xs uppercase hover:scale-105 active:scale-95 transition-transform">
                        "Initialize Engine"
                    </button>
                    <div class="absolute right-[-20px] bottom-[-20px] opacity-10 group-hover:scale-110 transition-transform duration-700 pointer-events-none">
                        <span class="material-symbols-outlined text-[200px]">"memory"</span>
                    </div>
                </div>
                // TODO (Module 2 — RES-001..RES-007): clicking "Summarize" pre-selects
                // the "Summarize" action in EditorView's control panel and navigates there.
                // Wire: on:click=move |_| set_active_view.set(View::Editor)
                <ActionCard icon="summarize"  icon_color="text-action"       title="Summarize"    desc="Executive distillation of core concepts."/>
                // TODO (Module 3 — GEN-001): pre-select "Press Release" action in EditorView.
                // Wire: on:click=move |_| set_active_view.set(View::Editor)
                <ActionCard icon="campaign"   icon_color="text-action"       title="Press Release" desc="Draft professional public communications."/>
                // TODO (Module CHA — CHA-001): navigate to Chat view.
                // Wire: on:click=move |_| set_active_view.set(View::Chat)
                <ActionCard icon="forum"      icon_color="text-[#66a6ea]"    title="Chat with Doc" desc="Direct query dialogue with your source text."/>
            </div>

            // ── Recent Transformations Table ───────────────────────────────────
            // Shows the N most recent processing operations across all projects.
            // TODO: Fetch from GET /api/projects?limit=10&sort=updated_at (TRA-001).
            // Each row is clickable to re-open that project in EditorView with the
            // result pre-loaded in the output panel.
            // TODO (EXO-001..EXO-006): the download icon should open an export modal
            // with format options: DOCX, PDF, HTML, Markdown, clipboard (copy).
            <div class="space-y-4">
                <div class="flex justify-between items-end">
                    <div>
                        <h3 class="font-sans font-black text-xl text-primary tracking-tighter">
                            "Recent Transformations"
                        </h3>
                        <p class="font-serif italic text-on-surf-var text-sm">
                            "History of processed intelligence"
                        </p>
                    </div>
                    <button class="text-xs font-bold uppercase tracking-widest text-primary flex items-center gap-1 hover:underline">
                        "View Full Archive"
                        <span class="material-symbols-outlined text-[16px]">"arrow_forward"</span>
                    </button>
                </div>
                <div class="bg-white rounded-xl shadow-sm overflow-hidden border border-slate-200/50">
                    <table class="w-full text-left border-collapse">
                        <thead>
                            <tr class="bg-surf-low border-b border-slate-200">
                                <Th>"Document Name"</Th>
                                <Th>"Transformation Type"</Th>
                                <Th>"Timestamp"</Th>
                                <th class="px-6 py-4 font-sans font-bold text-[10px] uppercase tracking-widest text-slate-500 text-right">
                                    "Actions"
                                </th>
                            </tr>
                        </thead>
                        <tbody class="divide-y divide-slate-100">
                            {TRANSFORMATIONS.iter().map(|t| view! {
                                <TransformationRow t=t.clone()/>
                            }).collect_view()}
                        </tbody>
                    </table>
                </div>
            </div>

            // ── Footer ─────────────────────────────────────────────────────────
            // Model signature and emergency controls.
            // TODO (SEC-002 — AES-256): "Emergency Purge" should call DELETE /api/purge
            // which wipes ~/.local-ai/workspace/ and the SQLite audit log, then
            // shows a confirmation banner. Require a two-step confirmation dialog.
            <footer class="pb-8 border-t border-slate-200 pt-8">
                <div class="flex flex-col md:flex-row justify-between items-center gap-6">
                    <div class="flex items-center gap-8">
                        <div class="flex flex-col">
                            <span class="text-[10px] font-bold uppercase tracking-widest text-slate-400">
                                "Computational Engine"
                            </span>
                            // TODO: populate from GET /api/settings → llm_model field
                            <span class="font-sans font-bold text-primary">"Qwen 3.5 — 32B Instruct"</span>
                        </div>
                        <div class="h-8 w-[1px] bg-slate-200 hidden md:block"></div>
                        <div class="flex flex-col">
                            <span class="text-[10px] font-bold uppercase tracking-widest text-slate-400">
                                "Privacy Status"
                            </span>
                            <span class="font-sans font-bold text-primary flex items-center gap-2">
                                <span class="w-2 h-2 bg-green-500 rounded-full"></span>
                                "100% Air-Gapped"
                            </span>
                        </div>
                    </div>
                    <div class="flex gap-4">
                        // TODO (TRA-003): export the full audit log as JSON/CSV
                        <button class="px-4 py-2 text-primary border border-primary rounded-lg text-xs font-bold uppercase tracking-widest hover:bg-primary hover:text-white transition-all">
                            "Export Audit Log"
                        </button>
                        <button class="px-4 py-2 bg-primary text-white rounded-lg text-xs font-bold uppercase tracking-widest active:opacity-80 transition-all">
                            "Emergency Purge"
                        </button>
                    </div>
                </div>
            </footer>
        </section>

        // Floating Action Button — visible when scrolled down, shortcut to "New Document"
        // TODO: conditionally show only when the user has scrolled past the drop zone.
        <div class="fixed bottom-8 right-8 z-50">
            <button
                class="w-14 h-14 bg-primary text-white rounded-full flex items-center justify-center shadow-2xl hover:scale-110 active:scale-95 transition-all"
                on:click=move |_| set_active_view.set(View::Editor)
            >
                <span class="material-symbols-outlined text-[28px]">"add"</span>
            </button>
        </div>
    }
}

// Dashboard sub-components

#[component]
fn FileBtn(icon: &'static str, label: &'static str, on_click: impl Fn() + 'static) -> impl IntoView {
    view! {
        <button
            class="flex items-center gap-2 px-5 py-2.5 bg-surf-high rounded-lg font-bold text-xs uppercase tracking-wider text-primary hover:bg-surf-highest transition-colors"
            on:click=move |_| on_click()
        >
            <span class="material-symbols-outlined text-[18px]">{icon}</span>
            {label}
        </button>
    }
}

#[component]
fn ActionCard(
    icon:       &'static str,
    icon_color: &'static str,
    title:      &'static str,
    desc:       &'static str,
) -> impl IntoView {
    view! {
        // TODO: add on:click handler when navigation is wired (see usage-site comments above)
        <div class="bg-white p-6 rounded-xl shadow-sm border border-transparent hover:border-outline-var transition-all cursor-pointer group">
            <span class=format!("material-symbols-outlined {icon_color} mb-4 block")>{icon}</span>
            <h4 class="font-sans font-bold text-sm uppercase tracking-tight mb-1 text-primary">{title}</h4>
            <p class="text-on-surf-var text-xs font-serif italic">{desc}</p>
        </div>
    }
}

#[component]
fn Th(children: Children) -> impl IntoView {
    view! {
        <th class="px-6 py-4 font-sans font-bold text-[10px] uppercase tracking-widest text-slate-500">
            {children()}
        </th>
    }
}

#[component]
fn TransformationRow(t: Transformation) -> impl IntoView {
    view! {
        <tr class="hover:bg-surf-low transition-colors group">
            <td class="px-6 py-4">
                <div class="flex items-center gap-3">
                    <span class="material-symbols-outlined text-slate-400">"description"</span>
                    <span class="font-sans font-bold text-sm text-primary">{t.name}</span>
                </div>
            </td>
            <td class="px-6 py-4">
                <span class=format!("px-2.5 py-1 {} rounded text-[10px] font-black uppercase tracking-tighter", t.badge)>
                    {t.kind}
                </span>
            </td>
            <td class="px-6 py-4 text-xs text-on-surf-var">{t.ts}</td>
            <td class="px-6 py-4 text-right">
                <div class="flex justify-end gap-2 opacity-0 group-hover:opacity-100 transition-opacity">
                    <RowBtn icon="visibility"/>
                    <RowBtn icon="download"/>
                </div>
            </td>
        </tr>
    }
}

#[component]
fn RowBtn(icon: &'static str) -> impl IntoView {
    view! {
        <button class="p-1.5 hover:bg-surf-cont rounded-lg text-primary transition-colors">
            <span class="material-symbols-outlined text-[18px]">{icon}</span>
        </button>
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// VIEW: EDITOR (Workspace Core)
// ═══════════════════════════════════════════════════════════════════════════════
// The primary transformation workspace. Three-column layout:
//   Left  (45%, blue accent border): Source document — editable or read-only view
//   Middle (240px, neutral):         Control panel — action selector + parameters
//   Right (flex-1, orange accent):   Result area — AI-generated output
//
// This view implements the core loop of OLIV4600:
//   Source text → parametrized LLM prompt → formatted professional output
//
// Corresponds to SRS modules: 2 (Summaries), 3 (Derived Content), 5 (Tone),
// 6 (Assisted Editing), 12 (Templates), 13 (Verifiability), 14 (Safe Publication).

#[component]
fn EditorView(set_active_view: WriteSignal<View>) -> impl IntoView {
    let ctx = use_context::<DocumentCtx>().expect("DocumentCtx");

    // ── Parámetros del panel de control ──────────────────────────────────────
    let (selected_action, set_selected_action) = signal("executive_summary".to_string());
    let (length_words,    set_length)          = signal(250u32);
    let (tone,            set_tone)            = signal(4u32);
    let (audience,        set_audience)        = signal("technical".to_string());
    let (language,        set_language)        = signal("es".to_string());
    let _ = set_language; // expuesto en la UI futura (TON-002)

    // Señal derivada: ¿hay resultado para mostrar?
    let has_output = move || !ctx.output.get().is_empty();

    view! {
        <div class="h-full flex overflow-hidden">

            // ── Column 1: Source Document ──────────────────────────────────────
            // The left panel displays the loaded source document.
            // The 4px blue left border visually signals "raw input / authority source".
            //
            // TODO (ING-003): When no file is loaded, render a contenteditable rich-text
            // editor here instead of static text. Support markdown shortcuts and basic
            // formatting (bold, italic, headers, lists) via a lightweight JS interop layer.
            //
            // TODO (Module 6 — EDI-001..EDI-007): The text should be spell-checked
            // in real time (underline squiggly on errors). Right-click context menu
            // should offer: "Simplify", "Reformulate", "Detect ambiguity" for the
            // selected sentence/paragraph.
            // ── Columna 1: Documento Fuente ───────────────────────────────────
            <section class="w-[45%] flex flex-col border-l-[4px] border-[#2E75B6] bg-white">
                // Cabecera del panel fuente
                <div class="h-12 border-b border-slate-100 flex items-center justify-between px-6 bg-surf-low/30">
                    <div class="flex items-center gap-4">
                        <span class="text-[10px] font-bold uppercase tracking-tighter text-outline">
                            {move || {
                                let f = ctx.filename.get();
                                if f.is_empty() { "Fuente: sin documento".to_string() }
                                else { format!("Fuente: {f}") }
                            }}
                        </span>
                        {move || (!ctx.filename.get().is_empty()).then(|| view! {
                            <span class="px-2 py-0.5 bg-[#b6d4fe]/30 text-[#436084] text-[9px] font-black rounded-sm">
                                "ES"
                            </span>
                        })}
                    </div>
                    <div class="flex gap-2 items-center">
                        // Botón para cargar nuevo documento
                        <button
                            class="text-[10px] font-bold uppercase tracking-tighter text-outline hover:text-primary flex items-center gap-1"
                            on:click=move |_| set_active_view.set(View::Dashboard)
                        >
                            <span class="material-symbols-outlined text-sm">"upload_file"</span>
                            "Cargar"
                        </button>
                    </div>
                </div>

                // Área de texto del documento
                // Si está vacío muestra editor; si tiene texto lo muestra como read-only
                {move || {
                    let text = ctx.text.get();
                    if text.is_empty() {
                        view! {
                            // ING-003: editor en blanco (textarea)
                            <textarea
                                class="flex-1 w-full p-12 font-serif text-lg leading-relaxed text-on-surf/90 resize-none border-none focus:ring-0 bg-white placeholder:text-slate-300"
                                placeholder="Escribe o pega el texto del documento aquí..."
                                on:input=move |ev| {
                                    ctx.text.set(event_target_value(&ev));
                                    ctx.filename.set("Documento sin título".to_string());
                                    ctx.word_count.set(
                                        ctx.text.get_untracked().split_whitespace().count() as u32
                                    );
                                }
                            />
                        }.into_any()
                    } else {
                        view! {
                            <div class="flex-1 overflow-y-auto p-12 font-serif text-base leading-relaxed text-on-surf/90 whitespace-pre-wrap">
                                {text}
                            </div>
                        }.into_any()
                    }
                }}

                // Pie del panel — estadísticas del documento
                <div class="h-10 bg-surf-low border-t border-slate-100 flex items-center justify-between px-6 shrink-0">
                    <div class="flex gap-4 text-[10px] font-bold uppercase text-outline">
                        <span>{move || format!("Palabras: {}", ctx.word_count.get())}</span>
                        <span>{move || {
                            let mins = (ctx.word_count.get() / 200).max(1);
                            format!("Lectura: ~{mins} min")
                        }}</span>
                    </div>
                    <div class="text-[10px] font-bold uppercase text-outline">"UTF-8"</div>
                </div>
            </section>

            // ── Column 2: Control Panel ────────────────────────────────────────
            // Parametrizes the transformation. All controls here feed into the
            // LLM prompt template system (§8.3 of SRS).
            //
            // TODO (Module 12 — PLT-001): "Action Selector" should be a proper
            // dropdown/accordion grouping all 18 SRS modules by category:
            //   Summarize       → Modules 2 (RES-001..RES-009)
            //   Communication   → Module 3 (GEN-001..GEN-011)
            //   Admin           → Module 4 (ADM-001..ADM-009)
            //   Analysis        → Modules 7,8,9,10
            //   Verifiability   → Modules 13,14
            <section class="w-[240px] bg-surf-low flex flex-col border-x border-slate-200/50">
                <div class="p-6 space-y-8 flex-1 overflow-y-auto">

                    // Action selector — todos los módulos SRS agrupados (RES, GEN, ADM, etc.)
                    <div>
                        <label class="block text-[10px] font-black uppercase tracking-widest text-on-surf-var mb-2">
                            "Action Selector"
                        </label>
                        <select
                            class="w-full bg-[#001b30] text-white border-none text-[11px] font-bold py-2 px-2 focus:ring-0 rounded"
                            on:change=move |ev| set_selected_action.set(event_target_value(&ev))
                        >
                            <optgroup label="── Resúmenes ──">
                                <option value="executive_summary" selected>"Resumen Ejecutivo"</option>
                                <option value="technical_summary">"Resumen Técnico"</option>
                                <option value="divulgative_summary">"Resumen Divulgativo"</option>
                                <option value="bullet_summary">"Puntos Clave"</option>
                                <option value="chronological_summary">"Resumen Cronológico"</option>
                                <option value="conclusions_summary">"Conclusiones y Recomendaciones"</option>
                                <option value="briefing_2min">"Briefing 2 min"</option>
                            </optgroup>
                            <optgroup label="── Comunicación ──">
                                <option value="press_release">"Nota de Prensa"</option>
                                <option value="headlines">"Titulares"</option>
                                <option value="linkedin_post">"Post LinkedIn"</option>
                                <option value="twitter_thread">"Hilo Twitter/X"</option>
                                <option value="blog_article">"Artículo de Blog"</option>
                                <option value="instagram_post">"Post Instagram"</option>
                                <option value="email_newsletter">"Email / Newsletter"</option>
                                <option value="speech">"Discurso"</option>
                                <option value="faqs">"FAQs"</option>
                                <option value="one_pager">"One-Pager / Ficha Resumen"</option>
                            </optgroup>
                            <optgroup label="── Administración ──">
                                <option value="key_quotes">"Citas Textuales"</option>
                                <option value="official_report">"Informe Oficial"</option>
                                <option value="meeting_minutes">"Acta de Reunión"</option>
                                <option value="administrative_resolution">"Resolución Administrativa"</option>
                                <option value="internal_memo">"Memorando Interno"</option>
                                <option value="allegations_response">"Alegaciones / Negociación"</option>
                            </optgroup>
                            <optgroup label="── Edición ──">
                                <option value="extract_commitments">"Compromisos Verificables"</option>
                                <option value="rewrite_formal">"Reescritura Formal"</option>
                                <option value="rewrite_shorter">"Reescritura Concisa"</option>
                                <option value="rewrite_persuasive">"Reescritura Persuasiva"</option>
                                <option value="rewrite_clearer">"Reescritura Clara"</option>
                                <option value="detect_redundancies">"Detectar Redundancias"</option>
                                <option value="translate_language">"Traducción"</option>
                                <option value="sentiment_analysis">"Análisis de Sentimiento"</option>
                                <option value="grammar_check">"Corrección Gramatical"</option>
                                <option value="simplify">"Simplificar (Lenguaje Claro)"</option>
                                <option value="detect_inconsistencies">"Detectar Inconsistencias"</option>
                                <option value="reformulate_paragraph">"Reformular Párrafo"</option>
                                <option value="detect_ambiguities">"Detectar Ambigüedades"</option>
                                <option value="improve_suggestions">"Sugerencias de Mejora"</option>
                                <option value="readability_analysis">"Análisis de Legibilidad"</option>
                                <option value="detect_evasive_language">"Lenguaje Evasivo"</option>
                            </optgroup>
                            <optgroup label="── Inteligencia ──">
                                <option value="semantic_versioning">"Versionado Semántico"</option>
                                <option value="merge_documents">"Fusión de Documentos"</option>
                                <option value="semantic_diff">"Diferencial Semántico"</option>
                                <option value="document_intersection">"Intersección Documental"</option>
                                <option value="detect_contradictions">"Detectar Contradicciones"</option>
                                <option value="versions_compare">"Comparar Versiones"</option>
                                <option value="inverse_questions">"Preguntas Inversas"</option>
                                <option value="press_release_check">"Verificar Nota de Prensa"</option>
                                <option value="validation_questions">"Checklist de Validación"</option>
                            </optgroup>
                            <optgroup label="── Extracción ──">
                                <option value="ner_extraction">"Entidades (NER)"</option>
                                <option value="keywords_extraction">"Palabras Clave"</option>
                                <option value="event_timeline">"Línea Temporal"</option>
                                <option value="impact_analysis">"Análisis de Impacto"</option>
                                <option value="verifiability_check">"Verificabilidad"</option>
                                <option value="evidence_gaps">"Huecos de Evidencia"</option>
                                <option value="traceability_map">"Mapa de Trazabilidad"</option>
                            </optgroup>
                            <optgroup label="── Privacidad ──">
                                <option value="anonymize">"Anonimización / Expurgo"</option>
                                <option value="preflight_check">"Preflight Documental"</option>
                                <option value="public_version">"Versión Pública"</option>
                                <option value="rgpd_check">"Verificación RGPD/LOPDGDD"</option>
                                <option value="style_linting">"Linting Documental"</option>
                                <option value="reader_simulation">"Simulador de Lector"</option>
                                <option value="generate_from_form">"Generar desde Formulario"</option>
                                <option value="generate_file_package">"Paquete de Expediente"</option>
                            </optgroup>
                            <optgroup label="── Crisis ──">
                                <option value="crisis_press_questions">"Simulacro Comparecencia"</option>
                                <option value="crisis_communication">"Kit de Crisis Reputacional"</option>
                                <option value="argumentario">"Argumentario"</option>
                                <option value="difficult_questions_simulator">"Simulador Preguntas Difíciles"</option>
                            </optgroup>
                        </select>
                    </div>

                    // Output length slider (RES-007: 50–500 words)
                    <div class="space-y-6">
                        <div>
                            <div class="flex justify-between mb-2">
                                <label class="text-[10px] font-black uppercase text-on-surf-var">"Length"</label>
                                <span class="text-[10px] font-bold text-primary">
                                    {move || format!("{}w", length_words.get())}
                                </span>
                            </div>
                            <input
                                class="w-full accent-primary"
                                type="range" min="50" max="500"
                                prop:value={move || length_words.get()}
                                style="height:2px; background:#e1e3e4;"
                                on:input=move |ev| {
                                    if let Ok(v) = event_target_value(&ev).parse::<u32>() {
                                        set_length.set(v);
                                    }
                                }
                            />
                        </div>

                        // Tone slider (TON-001: 1=Coloquial … 5=Formal Institucional)
                        <div>
                            <div class="flex justify-between mb-2">
                                <label class="text-[10px] font-black uppercase text-on-surf-var">"Tone"</label>
                                <span class="text-[10px] font-bold text-primary">
                                    {move || tone_label(tone.get())}
                                </span>
                            </div>
                            <input
                                class="w-full accent-primary"
                                type="range" min="1" max="5"
                                prop:value={move || tone.get()}
                                style="height:2px; background:#e1e3e4;"
                                on:input=move |ev| {
                                    if let Ok(v) = event_target_value(&ev).parse::<u32>() {
                                        set_tone.set(v);
                                    }
                                }
                            />
                        </div>

                        // Audience selector (RES-008) — valor inyectado en {publico_objetivo}
                        <div>
                            <label class="block text-[10px] font-black uppercase text-on-surf-var mb-2">
                                "Audience"
                            </label>
                            <select
                                class="w-full bg-surf-highest border-none text-[11px] font-bold py-2 focus:ring-0 rounded"
                                on:change=move |ev| set_audience.set(event_target_value(&ev))
                            >
                                <option value="citizen">"Ciudad / Ciudadanía"</option>
                                <option value="press">"Prensa / Global"</option>
                                <option value="technical" selected>"Técnico / Interno"</option>
                                <option value="executive">"Ejecutivo / Decisor"</option>
                            </select>
                        </div>
                    </div>
                </div>

                // Generate button — the primary CTA of the editor
                // TODO: on click → POST /api/transform with all parameters.
                // Show a streaming SSE response in the right panel using an EventSource.
                // Display token-by-token streaming with a blinking cursor.
                // On completion: store result in project history (TRA-001).
                <div class="p-4 bg-[#001b30]">
                    <button class="w-full bg-[#2E75B6] hover:bg-[#66a6ea] active:scale-[0.98] transition-all text-white py-4 rounded-sm flex flex-col items-center justify-center gap-1 group shadow-[0_0_20px_rgba(46,117,182,0.3)]">
                        <span class="material-symbols-outlined text-2xl group-active:animate-pulse">"bolt"</span>
                        <span class="text-[11px] font-black uppercase tracking-widest">"Generate"</span>
                    </button>
                    // TODO: compute actual inference latency from last response
                    <p class="text-[8px] text-center mt-3 text-slate-500 font-bold uppercase tracking-tight">
                        "System Latency: 42ms"
                    </p>
                </div>
            </section>

            // ── Column 3: Result Area ──────────────────────────────────────────
            // Displays the AI-generated output. The 4px orange left border visually
            // signals "processed / refined output" — the "right side" of the
            // Source→Result layout described in DESIGN.md §5.
            //
            // TODO (Module 13 — VER-001): Each sentence in the result should be
            // color-coded based on its traceability classification:
            //   Green  = substantiated by source text
            //   Amber  = inferred by the AI
            //   Red    = unsupported / uncertain
            // This data comes from a secondary "verifiability pass" prompt run
            // after the main generation.
            //
            // TODO (Module 14 — PUB-004): After generation, automatically run a
            // "preflight" check: undefined acronyms, ambiguous dates, inconsistent
            // names, figures without units, wrong tone for the selected channel.
            // Display results as inline annotations.
            <section class="flex-1 flex flex-col border-l-[4px] border-[#C45911] bg-surface">
                // Result header
                <div class="h-12 border-b border-slate-100 flex items-center justify-between px-6 bg-surf-highest/30">
                    <div class="flex items-center gap-4">
                        <span class="text-[10px] font-bold uppercase tracking-tighter text-[#C45911]">
                            "Output: Executive Summary"
                        </span>
                    </div>
                    <div class="flex gap-4">
                        // TODO (EXO-001): copy formatted text to clipboard
                        <button class="flex items-center gap-1 text-[10px] font-bold uppercase text-outline hover:text-primary transition-colors">
                            <span class="material-symbols-outlined text-sm">"content_copy"</span>
                            " Copy"
                        </button>
                        // TODO (EXO-002..EXO-005): export modal — DOCX, PDF, HTML, Markdown
                        <button class="flex items-center gap-1 text-[10px] font-bold uppercase text-outline hover:text-primary transition-colors">
                            <span class="material-symbols-outlined text-sm">"ios_share"</span>
                            " Export"
                        </button>
                    </div>
                </div>

                // Generated text display
                <div class="flex-1 p-12 overflow-y-auto">
                    {move || if show_result.get() {
                        view! {
                            <div class="max-w-2xl">
                                // "Verified Result" badge — will be replaced by the
                                // verifiability confidence score (VER-004) once Module 13 is live.
                                <div class="mb-8 inline-block px-3 py-1 bg-[#ffdbcb] text-[#783100] text-[9px] font-black uppercase tracking-widest rounded-sm">
                                    "Verified Result"
                                </div>
                                <div class="font-serif italic text-2xl text-on-surf/80 leading-relaxed space-y-6">
                                    <p>
                                        "The transition of the OLIV4600 system toward sovereign intelligence represents
                                        a critical infrastructure milestone for 2026. This technical pivot ensures that
                                        sensitive defense data remains isolated from public cloud vectors while
                                        maintaining computational superiority."
                                    </p>
                                    <p>"Key pillars identified include:"</p>
                                    <ul class="list-none space-y-4 not-italic font-sans text-sm uppercase tracking-tight font-bold text-primary">
                                        <BulletItem text="Strategic autonomy in algorithmic execution."/>
                                        <BulletItem text="Complete physical data sovereignty within institutional perimeters."/>
                                        <BulletItem text="Human-in-the-loop validation for all AI-generated strategic narratives."/>
                                    </ul>
                                    <p>
                                        "By prioritizing traceability and local processing, the institution mitigates
                                        the risks associated with third-party dependencies and enhances the reliability
                                        of its defensive innovation framework."
                                    </p>
                                </div>
                            </div>
                        }.into_any()
                    } else {
                        view! {
                            // Empty state — shown before first generation
                            <div class="flex flex-col items-center justify-center h-full text-center opacity-40">
                                <span class="material-symbols-outlined text-[48px] text-primary mb-4">"bolt"</span>
                                <p class="font-serif italic text-xl text-primary">
                                    "Configure parameters and click Generate"
                                </p>
                            </div>
                        }.into_any()
                    }}
                </div>

                // Result footer — provenance metadata + actions
                // TODO (TRA-001): populate timestamp, prompt version, and SHA-256 hash
                // of the output from the server response. Store in SQLite for audit trail.
                // TODO (VER-005): "Edit Result" should put the output into an editable
                // contenteditable area and track human edits vs AI-generated text separately.
                <div class="p-6 bg-surf-low/50 border-t border-slate-100">
                    <div class="flex flex-wrap items-center gap-x-6 gap-y-2 opacity-60">
                        <div class="flex items-center gap-2">
                            <span class="material-symbols-outlined text-xs">"verified"</span>
                            <span class="text-[10px] font-bold uppercase tracking-tight">
                                "Generated with [Executive Summary]"
                            </span>
                        </div>
                        <span class="w-1 h-1 bg-outline rounded-full"></span>
                        <span class="text-[10px] font-bold uppercase tracking-tight">"Apr 08, 2026"</span>
                        <span class="w-1 h-1 bg-outline rounded-full"></span>
                        <span class="text-[10px] font-bold uppercase tracking-tight">"Prompt v1.2"</span>
                        <span class="w-1 h-1 bg-outline rounded-full"></span>
                        // TODO: real SHA-256 of output text (TRA-001)
                        <span class="text-[10px] font-bold uppercase tracking-tight">"SHA-256: 8f92...a3e1"</span>
                    </div>
                    <div class="mt-6 flex gap-3">
                        <button class="bg-surf-highest text-on-surf text-[10px] font-black uppercase px-6 py-2.5 hover:bg-outline-var transition-colors flex items-center gap-2">
                            <span class="material-symbols-outlined text-sm">"edit"</span>
                            " Edit Result"
                        </button>
                        // TODO: "Regenerate" should re-POST /api/transform with same parameters
                        // but a slightly varied temperature (adds entropy for diverse outputs).
                        <button class="border border-outline-var text-on-surf text-[10px] font-black uppercase px-6 py-2.5 hover:bg-surf-high transition-colors flex items-center gap-2">
                            <span class="material-symbols-outlined text-sm">"refresh"</span>
                            " Regenerate"
                        </button>
                    </div>
                </div>
            </section>

            // Floating status ribbon — anchored bottom-right
            // TODO: drive from live backend state (processing queue, active node ID)
            <div class="fixed bottom-6 right-6 z-50 pointer-events-none">
                <div class="bg-[#002542] text-[#66a6ea] px-4 py-2 border-l-4 border-[#C45911] flex items-center gap-4 shadow-2xl">
                    <div class="flex gap-1">
                        <div class="w-1 h-3 bg-white/20"></div>
                        <div class="w-1 h-3 bg-[#66a6ea]"></div>
                        <div class="w-1 h-3 bg-[#66a6ea]"></div>
                    </div>
                    <span class="text-[10px] font-black uppercase tracking-[0.2em]">
                        "Processing Secured: Local Node 04"
                    </span>
                </div>
            </div>
        </div>
    }
}

#[component]
fn ControlMenuBtn(label: &'static str) -> impl IntoView {
    view! {
        // TODO: add on:click handler when action selection is wired to state
        <button class="w-full text-left px-3 py-1.5 text-[11px] font-medium text-outline hover:bg-surf-highest transition-colors">
            {label}
        </button>
    }
}

#[component]
fn BulletItem(text: &'static str) -> impl IntoView {
    view! {
        <li class="flex items-start gap-3">
            <span class="w-1.5 h-1.5 bg-[#C45911] mt-1.5 shrink-0"></span>
            {text}
        </li>
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// VIEW: ANALYSIS (Document Analysis Dashboard)
// ═══════════════════════════════════════════════════════════════════════════════
// Forensic intelligence report for the currently loaded document.
// Uses an asymmetric bento grid to present multiple analytical dimensions
// simultaneously — readable at a glance, drillable on click.
//
// Corresponds to SRS modules:
//   7  (Forensic Text Analysis — FOR-001..FOR-005)
//   8  (Textual Arithmetic — ARI-001..ARI-006)
//   9  (Inverse Questions — INV-001..INV-005)
//   10 (Structured Extraction and Metadata — EXT-001..EXT-007)
//   5  (Tone / Sentiment — TON-005, TON-006)
//   13 (Verifiability — VER-001..VER-005)
//
// TODO: Trigger a full analysis run on document load via POST /api/analyze.
// The server runs all analysis modules in parallel (goroutines / async tasks)
// and returns a structured AnalysisReport JSON. Store in SQLite for the audit log.
// All values shown here are currently static mockups.

#[component]
fn AnalysisView() -> impl IntoView {
    view! {
        <div class="p-10 max-w-7xl mx-auto">

            // ── Page Header ────────────────────────────────────────────────────
            <header class="mb-10">
                <div class="flex justify-between items-end">
                    <div>
                        <h2 class="text-4xl font-sans font-black tracking-tighter text-primary uppercase">
                            "Document Analysis"
                        </h2>
                        // TODO: display loaded document name + analysis timestamp
                        <p class="font-serif italic text-xl text-outline mt-1">
                            "Sovereign Intelligence Report: v4.6.0.28"
                        </p>
                    </div>
                    <div class="flex items-center gap-3 px-4 py-2 bg-primary text-[#66a6ea] rounded-sm">
                        <span class="material-symbols-outlined text-[20px]">"lock"</span>
                        <span class="text-[11px] font-bold uppercase tracking-[0.2em]">
                            "100% Offline Environment"
                        </span>
                    </div>
                </div>
            </header>

            // ── Bento Grid ─────────────────────────────────────────────────────
            <div class="grid grid-cols-12 gap-6">

                // Card: Readability Profile (FOR-001)
                // Flesch-Szigriszt index circular gauge.
                // TODO: compute via a Rust implementation of the Flesch-Szigriszt formula
                // on the source text. Also show: avg sentence length, lexical density.
                // Target for institutional documents: score 40–60 (university level).
                <div class="col-span-12 lg:col-span-4 bg-white p-8 shadow-sm rounded-lg">
                    <div class="flex justify-between items-start mb-8">
                        <h3 class="font-sans font-bold text-xs uppercase tracking-widest text-primary">
                            "Readability Profile"
                        </h3>
                        <span class="material-symbols-outlined text-primary/30">"menu_book"</span>
                    </div>
                    <div class="flex flex-col items-center py-4">
                        // SVG circular gauge — stroke-dasharray controls fill level
                        // TODO: calculate stroke-dashoffset from actual Flesch score (0–100)
                        // Formula: dashoffset = 502 * (1 - score/100)
                        <div class="relative w-48 h-48 flex items-center justify-center">
                            <svg class="w-full h-full -rotate-90">
                                <circle
                                    class="text-surf-high"
                                    cx="96" cy="96" r="80" fill="transparent"
                                    stroke="currentColor" stroke-width="12"
                                />
                                <circle
                                    class="text-action"
                                    cx="96" cy="96" r="80" fill="transparent"
                                    stroke="currentColor" stroke-width="12"
                                    stroke-dasharray="502"
                                    stroke-dashoffset="150"
                                />
                            </svg>
                            <div class="absolute inset-0 flex flex-col items-center justify-center">
                                <span class="text-4xl font-black text-primary">"72.4"</span>
                                <span class="text-[10px] font-bold uppercase text-outline tracking-tighter">
                                    "Flesch-Szigriszt"
                                </span>
                            </div>
                        </div>
                        <div class="mt-8 text-center">
                            <div class="px-4 py-1.5 bg-[#003b65] text-[#66a6ea] inline-block rounded-sm mb-2">
                                <span class="text-xs font-bold uppercase tracking-widest">"University Level"</span>
                            </div>
                            <p class="font-serif text-sm text-on-surf-var max-w-[200px] leading-relaxed">
                                "Highly technical syntax with academic structural patterns detected."
                            </p>
                        </div>
                    </div>
                </div>

                // Card: Emotional Resonance / Tone Thermometer (TON-005, TON-006)
                // A horizontal slider indicating the emotional tone of the document.
                // TODO: derive from sentiment analysis via the LLM.
                // Prompt: "Classify the overall tone of this text on a scale from
                // Hostile → Formal/Neutral → Advocacy. Return a 0-100 score."
                // Also show: top 3 detected emotions with confidence percentages.
                <div class="col-span-12 lg:col-span-4 bg-white p-8 shadow-sm rounded-lg flex flex-col">
                    <div class="flex justify-between items-start mb-8">
                        <h3 class="font-sans font-bold text-xs uppercase tracking-widest text-primary">
                            "Emotional Resonance"
                        </h3>
                        <span class="material-symbols-outlined text-primary/30">"psychology"</span>
                    </div>
                    <div class="flex-grow flex flex-col justify-center">
                        // Thermometer bar — position of the marker at 72% = "Formal/Neutral"
                        // TODO: bind position to sentiment score from analysis API
                        <div class="w-full h-4 bg-surf-high rounded-full overflow-hidden relative mb-4">
                            <div class="absolute inset-y-0 left-0 bg-primary w-[75%] rounded-full opacity-10"></div>
                            <div class="absolute inset-y-0 left-[72%] w-1 bg-[#C45911] shadow-[0_0_8px_rgba(196,89,17,0.5)] z-10"></div>
                        </div>
                        <div class="flex justify-between text-[10px] font-bold uppercase tracking-widest text-outline">
                            <span>"Hostile"</span>
                            <span class="text-primary">"Formal / Neutral"</span>
                            <span>"Advocacy"</span>
                        </div>
                    </div>
                    <div class="mt-8 p-4 bg-surface rounded-sm">
                        <div class="flex items-center gap-3">
                            <span class="material-symbols-outlined text-primary text-lg">"verified_user"</span>
                            <div>
                                <span class="block text-[11px] font-black uppercase text-primary">
                                    "Objective Profile"
                                </span>
                                <p class="font-serif text-sm italic">
                                    "94% compliance with institutional tone."
                                </p>
                            </div>
                        </div>
                    </div>
                </div>

                // Card: Critical Anomalies (FOR-002, EDI-003, EDI-006, INV-001..INV-005)
                // High-priority issues detected in the document: evasive language,
                // data gaps, ambiguous commitments, missing evidence.
                // Uses the dark tertiary background (#401700) for immediate visual urgency.
                // TODO: populate from the Inverse Questions engine (Module 9):
                //   - FOR-002: flag passive-voice liability clauses
                //   - EDI-006: "reasonable efforts" with no quantified metrics
                //   - INV-001: mandatory fields missing from press releases
                //   - VER-003: assertions of impact with no data source
                <div class="col-span-12 lg:col-span-4 bg-[#401700] p-8 shadow-sm rounded-lg text-white">
                    <div class="flex justify-between items-start mb-6">
                        <h3 class="font-sans font-bold text-xs uppercase tracking-widest text-[#ffdbcb]">
                            "Critical Anomalies"
                        </h3>
                        <span class="material-symbols-outlined text-[#fa813a]">"report"</span>
                    </div>
                    <ul class="space-y-4">
                        <AnomalyItem
                            title="Evasive Language"
                            desc="Multiple instances of passive voice in liability clauses."
                        />
                        <AnomalyItem
                            title="Data Gap"
                            desc="Fiscal year 2024 projections missing from summary."
                        />
                        <AnomalyItem
                            title="Ambiguity Alert"
                            desc="\"Reasonable efforts\" clause lacks quantified metrics."
                        />
                    </ul>
                </div>

                // Card: Named Entity Recognition Table (EXT-001)
                // Extracts persons, organizations, dates, locations, amounts.
                // TODO: POST /api/analyze → NER step uses the LLM with a structured
                // extraction prompt, returning JSON: [{ entity, type, confidence }].
                // Render each row with the appropriate color-coded type badge.
                // The "Export CSV" button should trigger EXO-001/EXO-005 (copy/markdown).
                // TODO (EXT-006): below the NER table, add a collapsible "Event Timeline"
                // section that orders all DATE entities chronologically.
                <div class="col-span-12 lg:col-span-8 bg-white p-8 shadow-sm rounded-lg">
                    <div class="flex justify-between items-start mb-8">
                        <h3 class="font-sans font-bold text-xs uppercase tracking-widest text-primary">
                            "Named Entity Recognition (NER)"
                        </h3>
                        <button class="text-[10px] font-bold uppercase text-primary bg-primary/10 px-3 py-1 rounded-sm hover:bg-primary/20 transition-colors">
                            "Export CSV"
                        </button>
                    </div>
                    <table class="w-full text-left">
                        <thead>
                            <tr class="border-b border-outline-var/20">
                                <NerTh>"Type"</NerTh>
                                <NerTh>"Extracted Entity"</NerTh>
                                <NerTh>"Confidence"</NerTh>
                            </tr>
                        </thead>
                        <tbody class="divide-y divide-outline-var/10">
                            <NerRow entity_type="PERSON"       badge_class="bg-blue-100 text-blue-800"      entity="Dr. Julian Vane"     pct=98/>
                            <NerRow entity_type="ORGANIZATION" badge_class="bg-purple-100 text-purple-800"  entity="Aetheris Corp Int."  pct=92/>
                            <NerRow entity_type="DATE"         badge_class="bg-amber-100 text-amber-800"    entity="October 14, 2026"    pct=100/>
                            <NerRow entity_type="LOCATION"     badge_class="bg-emerald-100 text-emerald-800" entity="Geneva Free Port"   pct=87/>
                        </tbody>
                    </table>
                </div>

                // Card: Structural Metadata (EXT-003, EXT-004, EXT-005)
                // Keyword extraction, confidentiality suggestion, target audience detection.
                // TODO (EXT-003): POST /api/analyze → keywords step generates 5-10 thematic
                // keywords plus a suggested document category. Store in SQLite for library search.
                // TODO (EXT-004): confidentiality suggestion is LLM-classified:
                //   Public / Internal / Restricted / Confidential / Top Secret
                // TODO (EXT-005): audience detection → feeds back into Module 4 (Admin) checks.
                <div class="col-span-12 lg:col-span-4 space-y-6">
                    <div class="bg-white p-8 shadow-sm rounded-lg border-l-4 border-primary">
                        <h3 class="font-sans font-bold text-xs uppercase tracking-widest text-primary mb-6">
                            "Structural Metadata"
                        </h3>
                        <div class="space-y-6">
                            <div>
                                <label class="text-[10px] font-black uppercase text-outline block mb-2">
                                    "Primary Keywords"
                                </label>
                                <div class="flex flex-wrap gap-2">
                                    <KeywordChip label="Sovereignty"/>
                                    <KeywordChip label="Encryption"/>
                                    <KeywordChip label="Liability"/>
                                    <KeywordChip label="Protocol"/>
                                </div>
                            </div>
                            <div>
                                <label class="text-[10px] font-black uppercase text-outline block mb-2">
                                    "Confidentiality Level"
                                </label>
                                <div class="flex items-center gap-3">
                                    <div class="flex-grow h-2 bg-surf-high rounded-full overflow-hidden">
                                        <div class="w-4/5 h-full bg-[#C45911]"></div>
                                    </div>
                                    <span class="text-[11px] font-black text-[#C45911] uppercase">
                                        "Restricted"
                                    </span>
                                </div>
                            </div>
                            <div>
                                <label class="text-[10px] font-black uppercase text-outline block mb-2">
                                    "Target Audience"
                                </label>
                                <p class="font-serif italic text-sm text-primary">
                                    "Technical Steering Committee & Legal Oversight"
                                </p>
                            </div>
                        </div>
                    </div>
                    // Impact analysis placeholder (EXT-007)
                    // TODO: "Why does this matter? Who is affected?" — LLM prompt that
                    // produces a 2-3 sentence impact statement for the executive summary.
                    <div class="bg-surf-low p-6 rounded-lg">
                        <h4 class="text-[10px] font-black uppercase text-outline mb-3">"Impact Analysis"</h4>
                        <p class="font-serif italic text-sm text-primary/60">
                            "Run analysis to generate impact assessment..."
                        </p>
                    </div>
                </div>

                // Card: Event Timeline (EXT-006)
                // Chronological ordering of all DATE entities extracted by NER.
                // The active node (current analysis date) is highlighted in action orange.
                // TODO: sort DATE entities chronologically → render as a horizontal
                // timeline with absolute positioning. Future dates shown at 40% opacity.
                <div class="col-span-12 bg-white p-8 shadow-sm rounded-lg">
                    <div class="flex justify-between items-start mb-10">
                        <h3 class="font-sans font-bold text-xs uppercase tracking-widest text-primary">
                            "Document Event Timeline"
                        </h3>
                        <span class="material-symbols-outlined text-primary/30">"timeline"</span>
                    </div>
                    <div class="relative py-10">
                        // Horizontal axis line
                        <div class="absolute top-1/2 left-0 w-full h-px bg-outline-var/30 -translate-y-1/2"></div>
                        <div class="flex justify-between relative px-10">
                            <TimelineNode date="2021-Q3" label="Initial Protocol Drafted" active=false/>
                            <TimelineNode date="2023-M11" label="Aetheris Acquisition" active=false/>
                            <TimelineNode date="2026-OCT" label="Current Analysis Horizon" active=true/>
                            <TimelineNode date="2027-PROJ" label="Sunsetting Phase" active=false/>
                        </div>
                    </div>
                </div>
            </div>

            // Footer
            <footer class="mt-12 flex justify-between items-center text-outline-var">
                <p class="text-[10px] font-bold uppercase tracking-widest">
                    "© 2026 OLIV4600 SOVEREIGN SYSTEMS"
                </p>
                <div class="flex gap-4">
                    // TODO: wire to real metrics from backend (inference time, node ID)
                    <span class="text-[10px] font-bold uppercase tracking-widest">"LATENCY: 12ms"</span>
                    <span class="text-[10px] font-bold uppercase tracking-widest">"NODE: LOCAL-GEN-01"</span>
                </div>
            </footer>
        </div>
    }
}

// Analysis sub-components

#[component]
fn AnomalyItem(title: &'static str, desc: &'static str) -> impl IntoView {
    view! {
        <li class="flex gap-4 items-start pb-4 border-b border-white/10 last:border-0 last:pb-0">
            <div class="w-2 h-2 rounded-full bg-[#fa813a] mt-1.5 shrink-0"></div>
            <div>
                <h4 class="text-[11px] font-black uppercase tracking-widest text-[#fa813a]">{title}</h4>
                <p class="font-serif text-sm text-white/80 leading-snug mt-1">{desc}</p>
            </div>
        </li>
    }
}

#[component]
fn NerTh(children: Children) -> impl IntoView {
    view! {
        <th class="pb-4 text-[11px] font-black uppercase text-outline tracking-widest">
            {children()}
        </th>
    }
}

#[component]
fn NerRow(
    entity_type:  &'static str,
    badge_class:  &'static str,
    entity:       &'static str,
    pct:          u32,
) -> impl IntoView {
    let bar_w = format!("w-[{}%]", pct);
    view! {
        <tr>
            <td class="py-4">
                <span class=format!("px-2 py-0.5 {} text-[10px] font-bold uppercase rounded-sm", badge_class)>
                    {entity_type}
                </span>
            </td>
            <td class="py-4 font-serif font-bold text-primary">{entity}</td>
            <td class="py-4">
                <div class="w-32 h-1.5 bg-surf-high rounded-full overflow-hidden">
                    <div class=format!("{bar_w} h-full bg-primary")></div>
                </div>
            </td>
        </tr>
    }
}

#[component]
fn KeywordChip(label: &'static str) -> impl IntoView {
    view! {
        <span class="text-[11px] font-bold px-2 py-1 bg-surf-high rounded-sm">{label}</span>
    }
}

#[component]
fn TimelineNode(date: &'static str, label: &'static str, active: bool) -> impl IntoView {
    let dot_class = if active {
        "w-6 h-6 rounded-full bg-[#C45911] border-4 border-white z-10 shadow-lg shadow-[#C45911]/20"
    } else {
        "w-4 h-4 rounded-full bg-primary border-4 border-white z-10"
    };
    let date_class = if active {
        "block text-[10px] font-black uppercase text-[#C45911]"
    } else {
        "block text-[10px] font-black uppercase text-primary"
    };
    view! {
        <div class=if active { "flex flex-col items-center" } else { "flex flex-col items-center opacity-40" }>
            <div class=dot_class></div>
            <div class="mt-4 text-center">
                <span class=date_class>{date}</span>
                <p class="font-serif text-sm mt-1 max-w-[120px]">{label}</p>
            </div>
        </div>
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// VIEW: CHAT (Document Chat)
// ═══════════════════════════════════════════════════════════════════════════════
// Free-form conversational interface over the loaded document.
// Split 50/50: source document on the left, chat thread on the right.
//
// Corresponds to SRS §5 (Chat over Document — CHA-001..CHA-004):
//   CHA-001: Free-form chat with full document context
//   CHA-002: Quick-question chips ("Explain like I'm 12", "What are the risks?")
//   CHA-003: Briefing mode ("Give me a 2-minute oral summary")
//   CHA-004: Session conversation history
//
// TODO: Chat messages are sent via POST /api/chat (the existing SSE stream endpoint).
// The request body includes: { doc_id, conversation_history[], user_message }
// The server prepends the full document text as system context (chunked if needed).
// Responses stream back token-by-token via SSE and are appended to the chat list.
//
// TODO (TRA-001): each chat exchange is logged to SQLite with timestamp, user query,
// model response, and context window usage. Visible in the Audit view.
//
// Highlighted passages in the source document (`.highlight-ref` class) correspond
// to text fragments cited in the AI's last response. This cross-panel linking
// requires the backend to return source offsets alongside the answer.

#[component]
fn ChatView() -> impl IntoView {
    // TODO: message list should be a Vec<ChatMessage> reactive signal, updated
    // as SSE tokens arrive. Auto-scroll the chat panel to the bottom on new messages.
    // TODO: input field should track keypresses; Shift+Enter for newline, Enter to send.

    view! {
        <div class="h-full flex overflow-hidden">

            // ── Left Panel: Document Viewer ────────────────────────────────────
            // Displays the source document in read-only mode with highlighted
            // passages that correspond to AI citations in the chat.
            // TODO: implement cross-panel citation linking:
            //   1. Each AI response includes a list of source_offsets (char start/end).
            //   2. On new AI message: scroll the document panel to the first cited passage
            //      and apply highlight-ref styling to each cited span.
            //   3. Clicking a highlighted passage in the document scrolls the chat to
            //      the message that cited it.
            <section class="w-1/2 bg-surf-low border-r border-slate-200 p-12 overflow-y-auto relative">
                <div class="max-w-2xl mx-auto bg-white shadow-sm p-16">
                    <div class="mb-12 border-b border-slate-100 pb-8">
                        // TODO: populate from loaded document metadata
                        <span class="text-[10px] font-bold text-slate-400 tracking-widest uppercase">
                            "Report ID: DEF-2026-X4"
                        </span>
                        <h2 class="font-serif text-4xl text-primary mt-2">
                            "Sovereign Defense Innovation 2026"
                        </h2>
                        <p class="text-xs text-on-surf-var mt-4 font-semibold italic">
                            "Classified: Internal Use Only — AI Augmented Audit"
                        </p>
                    </div>
                    // Document body with inline citation highlights
                    // Orange underlines mark passages referenced in the last AI response
                    <article class="font-serif text-lg leading-relaxed text-slate-800 space-y-6">
                        <p>
                            "The current landscape of defense procurement requires a fundamental shift toward "
                            <span class="bg-[#fa813a]/20 border-b-2 border-[#C45911] font-bold">
                                "decentralized manufacturing hubs"
                            </span>
                            " and modular intelligence systems. By 2026, the projected expenditure
                            for autonomous verification is expected to reach 14.2% of the total R&D budget."
                        </p>
                        <h3 class="font-sans font-bold text-sm uppercase tracking-tighter text-slate-400 mt-8">
                            "Strategic Analysis"
                        </h3>
                        <p>
                            "Our primary objective focuses on the integration of Sovereign LLMs into the
                            tactical decision-making cycle. Unlike previous iterations, "
                            <span class="bg-[#fa813a]/20 border-b-2 border-[#C45911] font-bold italic">
                                "the OLIV4600 engine ensures 100% data sovereignty by executing all
                                weights locally"
                            </span>
                            " within the hardened perimeter."
                        </p>
                        // Pull quote block — visually distinct section of cited content
                        <div class="bg-surf-cont p-6 my-8 rounded-lg border-l-4 border-primary">
                            <p class="italic text-slate-600">
                                "\"The architecture of the future is not built on more data,
                                but on more verifiable intelligence.\" — Strategic Memo X-12"
                            </p>
                        </div>
                        <p>
                            "Furthermore, the development of kinetic-response AI requires a rigorous ethical
                            framework that operates "
                            <span class="bg-[#fa813a]/20 border-b-2 border-[#C45911]">
                                "independent of external connectivity"
                            </span>
                            ". This \"Air-Gap Intelligence\" model is the cornerstone of our current
                            sovereign defense posture."
                        </p>
                    </article>
                </div>

                // Quick-question chips (CHA-002) — floating above the scroll area
                // These are preset prompts that bypass the input field and fire immediately.
                // TODO: on chip click → add the chip text as a user message → POST /api/chat
                <div class="sticky bottom-0 flex gap-2 flex-wrap pt-4 pb-2">
                    <QuickChip icon="child_care" label="Explain like I'm 12"/>
                    <QuickChip icon="warning"    label="What are the risks?"/>
                    <QuickChip icon="search"     label="What information is missing?"/>
                    // TODO (CHA-003): "2-minute briefing" chip → generates an oral summary
                    // formatted with speaker pause marks, suitable for reading aloud
                    <QuickChip icon="mic"        label="2-minute briefing"/>
                </div>
            </section>

            // ── Right Panel: Chat Interface ────────────────────────────────────
            <section class="w-1/2 flex flex-col bg-white">

                // Chat status ribbon — shows session metadata
                // TODO: populate thread ID from project context (unique per document session)
                // TODO (CHA-004): session history is stored in memory for the duration of
                // the browser session. For persistent history, POST /api/chat saves each
                // exchange to SQLite under the project_id. On project re-open, load history.
                <div class="h-12 border-b border-slate-100 flex items-center justify-between px-6 bg-surf-low/50">
                    <div class="flex items-center gap-4">
                        <div class="flex items-center gap-2">
                            <div class="w-2 h-2 rounded-full bg-emerald-500"></div>
                            <span class="text-[10px] font-black uppercase tracking-widest text-slate-500">
                                "Local Instance Online"
                            </span>
                        </div>
                        <div class="h-4 w-[1px] bg-slate-300"></div>
                        <span class="text-[10px] font-bold text-primary uppercase">
                            "Thread: Defense-Audit-X4"
                        </span>
                    </div>
                    <div class="flex gap-3">
                        // TODO: download exports the full conversation as a markdown transcript
                        <span class="material-symbols-outlined text-slate-400 text-sm cursor-pointer hover:text-primary">"download"</span>
                        <span class="material-symbols-outlined text-slate-400 text-sm cursor-pointer hover:text-primary">"more_vert"</span>
                    </div>
                </div>

                // Chat message thread
                <div class="flex-1 overflow-y-auto p-8 space-y-8">

                    // AI greeting message (shown on document load)
                    // TODO: generate this automatically when a document is first loaded:
                    // POST /api/chat with a special "init" message that triggers the model
                    // to describe what it found in the document and offer assistance.
                    <ChatBubbleAi
                        text="I've analyzed your document. What would you like to know about the sovereign defense architecture or the identified risk factors?"
                    />

                    // User message example
                    <ChatBubbleUser
                        text="Can you elaborate on the security benefits of the local engine mentioned in the analysis?"
                        time="14:22"
                    />

                    // AI detailed response
                    // TODO: structured responses (numbered lists, headers) should be rendered
                    // as rich HTML from a lightweight Markdown parser, not as raw text.
                    // TODO (VER-002): each cited fact should include a "§ source" link that,
                    // when clicked, scrolls the document panel to the referenced passage and
                    // highlights it with the .highlight-ref style.
                    <ChatBubbleAiDetailed/>
                </div>

                // Message input area
                // TODO: submit on Enter key (prevent newline), allow Shift+Enter for newline.
                // Show "OLIV4600 is thinking..." indicator while the SSE stream is open.
                // Display token count to help users stay within the context window (CHA-001).
                <div class="p-8 bg-white border-t border-slate-100">
                    <div class="relative flex items-end gap-4 bg-surf-low rounded-xl p-4 border border-slate-200 focus-within:border-primary transition-all">
                        <textarea
                            class="flex-1 bg-transparent border-none focus:ring-0 text-sm text-on-surf placeholder:text-slate-400 resize-none max-h-32"
                            placeholder="Ask the sovereign engine anything about this document..."
                            rows="2"
                        />
                        <div class="flex gap-2">
                            // TODO (ING-005): attach additional context documents to the
                            // conversation — uploaded files are appended to the context window
                            <button class="p-2 text-slate-400 hover:text-primary transition-colors">
                                <span class="material-symbols-outlined">"attach_file"</span>
                            </button>
                            // TODO: on click → read textarea value → POST /api/chat →
                            // open SSE stream → append tokens to chat panel in real time
                            <button class="bg-[#C45911] text-white p-3 rounded-lg shadow-md hover:bg-[#401700] transition-all active:scale-95">
                                <span class="material-symbols-outlined">"send"</span>
                            </button>
                        </div>
                    </div>
                    <div class="mt-4 flex justify-between items-center px-1">
                        <div class="flex items-center gap-2">
                            <span class="material-symbols-outlined text-[16px] text-emerald-500">"verified_user"</span>
                            <span class="text-[9px] font-bold uppercase tracking-widest text-slate-400">
                                "Secure Audit Mode Enabled"
                            </span>
                        </div>
                        // TODO: update this live as the conversation grows.
                        // When nearing the context limit, show a warning in amber.
                        <span class="text-[9px] font-bold uppercase tracking-widest text-slate-400">
                            "Context Window: 12.4k tokens"
                        </span>
                    </div>
                </div>
            </section>
        </div>
    }
}

// Chat sub-components

#[component]
fn QuickChip(icon: &'static str, label: &'static str) -> impl IntoView {
    view! {
        <button class="bg-white/90 backdrop-blur shadow-xl border border-slate-200 px-4 py-2 text-[11px] font-bold uppercase tracking-tight text-primary hover:bg-primary hover:text-white transition-all rounded-full flex items-center gap-2">
            <span class="material-symbols-outlined text-[14px]">{icon}</span>
            {label}
        </button>
    }
}

#[component]
fn ChatBubbleAi(text: &'static str) -> impl IntoView {
    view! {
        <div class="flex flex-col items-start max-w-[85%]">
            <div class="flex items-center gap-2 mb-2">
                <span class="w-6 h-6 bg-primary text-white flex items-center justify-center text-[10px] font-bold rounded">
                    "O"
                </span>
                <span class="text-[11px] font-bold uppercase text-slate-400 tracking-tight">
                    "OLIV4600 Engine"
                </span>
            </div>
            <div class="bg-surf-low p-6 rounded-xl rounded-tl-none border border-slate-100 group">
                <p class="font-serif text-lg text-slate-700 leading-snug">{text}</p>
                <button class="mt-4 flex items-center gap-2 text-[10px] font-bold uppercase tracking-widest text-primary opacity-0 group-hover:opacity-100 transition-opacity">
                    <span class="material-symbols-outlined text-xs">"export_notes"</span>
                    "Export Answer"
                </button>
            </div>
        </div>
    }
}

#[component]
fn ChatBubbleUser(text: &'static str, time: &'static str) -> impl IntoView {
    view! {
        <div class="flex flex-col items-end w-full">
            <div class="max-w-[75%] bg-primary text-white p-5 rounded-xl rounded-tr-none shadow-lg">
                <p class="text-sm leading-relaxed">{text}</p>
            </div>
            <span class="text-[10px] font-bold uppercase text-slate-300 mt-2 mr-1">
                {format!("Sent {time}")}
            </span>
        </div>
    }
}

#[component]
fn ChatBubbleAiDetailed() -> impl IntoView {
    view! {
        <div class="flex flex-col items-start max-w-[90%]">
            <div class="flex items-center gap-2 mb-2">
                <span class="w-6 h-6 bg-primary text-white flex items-center justify-center text-[10px] font-bold rounded">
                    "O"
                </span>
                <span class="text-[11px] font-bold uppercase text-slate-400 tracking-tight">
                    "OLIV4600 Engine"
                </span>
            </div>
            <div class="bg-surf-low p-6 rounded-xl rounded-tl-none border border-slate-100 group space-y-4">
                <p class="font-serif text-lg text-slate-700 leading-snug">
                    "Based on the highlighted sections of the report, the security benefits
                    of the local OLIV4600 engine are three-fold:"
                </p>
                <ul class="space-y-3 text-sm text-slate-600">
                    // TODO (VER-002): each numbered point should carry a source citation
                    // e.g. "— §2, para. 3" that links back to the document panel
                    <ChatAnswerPoint n="01" title="Data Sovereignty"
                        text="By operating within a hardened perimeter, no raw document data or inference
                              metadata ever traverses the public internet, mitigating the risk of MITM attacks."
                    />
                    <ChatAnswerPoint n="02" title="Latency Elimination"
                        text="Local execution ensures deterministic response times for kinetic-response
                              applications where millisecond delays are critical."
                    />
                    <ChatAnswerPoint n="03" title="Verification"
                        text="The engine supports hardware-level auditing, allowing human operators to verify
                              the exact compute paths used to generate an intelligence output."
                    />
                </ul>
                <div class="pt-4 flex gap-4">
                    <button class="flex items-center gap-2 text-[10px] font-bold uppercase tracking-widest text-primary">
                        <span class="material-symbols-outlined text-xs">"content_copy"</span>
                        "Copy"
                    </button>
                    <button class="flex items-center gap-2 text-[10px] font-bold uppercase tracking-widest text-primary">
                        <span class="material-symbols-outlined text-xs">"export_notes"</span>
                        "Export Answer"
                    </button>
                </div>
            </div>
        </div>
    }
}

#[component]
fn ChatAnswerPoint(n: &'static str, title: &'static str, text: &'static str) -> impl IntoView {
    view! {
        <li class="flex gap-3">
            <span class="font-bold text-primary shrink-0">{format!("{n}.")}</span>
            <span>
                <strong class="text-primary">{title}": "</strong>
                {text}
            </span>
        </li>
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// VIEW: PIPELINE (Production Pipeline / Verifiability)
// ═══════════════════════════════════════════════════════════════════════════════
// Visualizes the document derivation chain: from the root source document
// through all generated outputs (executive summary → press release → social posts).
// This is the "genealogy" view that proves every derived document's provenance.
//
// Corresponds to SRS modules:
//   11 (Document Production Chain — CAD-001..CAD-005)
//   17 (Expedition Package — EXP-001..EXP-005)
//   13 (Verifiability — VER-002, VER-005 traceability map export)
//   4.4 (Traceability — TRA-002 document genealogy)
//
// TODO (CAD-002): the pipeline should be configurable — users drag nodes to
// define the derivation chain: acta → convocatoria → nota de prensa → posts → FAQs.
// Each node is a transformation type from Module 3 or Module 4.
//
// TODO (CAD-003): "Propagate changes" — when the source document is updated,
// automatically flag all derived documents as "stale" (orange badge) and offer
// a one-click "Regenerate All" button to refresh the entire chain.
//
// TODO (EXP-005): "Export Package" → ZIP with all derived documents in a structured
// folder hierarchy: /expedition-{id}/source.docx, /summary.pdf, /press-release.docx, etc.

#[component]
fn PipelineView() -> impl IntoView {
    view! {
        <section class="p-12 flex flex-col min-h-full">

            // ── Header ─────────────────────────────────────────────────────────
            <div class="flex justify-between items-end mb-16">
                <div>
                    <h2 class="font-sans text-4xl font-black text-primary tracking-tight mb-2">
                        "Production Pipeline"
                    </h2>
                    <p class="font-serif text-slate-500 text-xl max-w-2xl">
                        "Visualizing the propagation of institutional intelligence from source to distribution nodes."
                    </p>
                </div>
                // TODO (CAD-002): "Regenerate Full Pipeline" → POST /api/pipeline/regenerate
                // with { project_id }. Server queues all pending transformation jobs and
                // streams progress events back. Rotate the icon while processing.
                <button class="group flex items-center gap-3 bg-[#401700] text-white px-8 py-4 rounded-lg font-sans font-bold text-sm tracking-widest uppercase hover:bg-[#622700] transition-all shadow-xl">
                    <span class="material-symbols-outlined group-hover:rotate-180 transition-transform duration-500">
                        "cached"
                    </span>
                    "Regenerate Full Pipeline"
                </button>
            </div>

            // ── Horizontal Pipeline Visualizer ─────────────────────────────────
            // Nodes are connected by horizontal lines (CSS ::after pseudo-element).
            // Each node represents either the root document or a derived output.
            // TODO: make this list dynamic — fetch nodes from GET /api/pipeline/{project_id}
            // which returns the derivation DAG from SQLite (TRA-002).
            // TODO (CAD-005): add a temporal sequencing view that shows:
            //   convocatoria → recordatorio → nota del día → resumen posterior
            // with scheduled send dates attached to each node.
            <div class="flex-1 flex items-center overflow-x-auto pb-12">
                <div class="flex gap-12 items-stretch">

                    // Node: Root Source Document
                    // The "ROOT AUTHORITY" — all derived documents trace back here.
                    // The "SOURCE UPDATED" badge appears when the source has been modified
                    // after derived documents were generated (CAD-003 propagation pending).
                    <PipelineSourceNode/>

                    // Node: Executive Summary (Derived Layer 01)
                    // Status: Edited — human has modified the AI output.
                    // TODO: "Edited" badge appears when the output has been manually changed
                    // after generation. Store a dirty flag in SQLite alongside the output.
                    <PipelineDerivedNode
                        layer="Derived Layer 01"
                        icon="summarize"
                        icon_color="text-blue-500"
                        accent_color="bg-blue-500"
                        title="Executive Summary"
                        desc="Synthesized high-level overview for leadership alignment."
                        status="Edited"
                        status_class="bg-blue-50 text-blue-600"
                        locked=false
                        waiting_for=""
                    />

                    // Node: Press Release (Derived Layer 02)
                    // Status: Generated — AI output not yet reviewed by human.
                    // TODO: "Review Now" CTA should navigate to EditorView with this
                    // specific derived document loaded in the result panel for editing.
                    <PipelineDerivedNode
                        layer="Derived Layer 02"
                        icon="news"
                        icon_color="text-emerald-500"
                        accent_color="bg-emerald-500"
                        title="Press Release"
                        desc="External-facing communiqué for global media distribution."
                        status="Generated"
                        status_class="bg-emerald-50 text-emerald-600"
                        locked=false
                        waiting_for=""
                    />

                    // Node: LinkedIn Post (Social Amplification)
                    // Status: Pending — waiting for Press Release (Layer 02) to be approved.
                    // TODO (CAD-002): enforce dependency order — don't generate social posts
                    // until the upstream press release has been reviewed (status = "Approved").
                    // This prevents propagating errors from the press release into social copy.
                    <PipelineDerivedNode
                        layer="Social Amplification"
                        icon="share"
                        icon_color="text-slate-400"
                        accent_color="bg-slate-300"
                        title="LinkedIn Post"
                        desc="Narrative-driven professional update for corporate ecosystem."
                        status="Pending"
                        status_class="bg-slate-200 text-slate-500"
                        locked=true
                        waiting_for="Awaiting Layer 02..."
                    />

                    // Node: Twitter/X Thread (Viral Extraction)
                    // Status: Pending — also waiting for Press Release approval.
                    // TODO (GEN-004): Twitter thread generation respects the 280-char limit
                    // per tweet. The LLM prompt includes: "Split this into N tweets,
                    // each ≤280 chars, numbered 1/N...N/N".
                    <PipelineDerivedNode
                        layer="Viral Extraction"
                        icon="rebase_edit"
                        icon_color="text-slate-400"
                        accent_color="bg-slate-300"
                        title="Twitter / X Thread"
                        desc="Concise, high-impact data points for rapid discourse."
                        status="Pending"
                        status_class="bg-slate-200 text-slate-500"
                        locked=true
                        waiting_for="Awaiting Layer 02..."
                    />

                    // TODO (GEN-007 + GEN-009): add more nodes as needed:
                    //   - Email Newsletter (GEN-007)
                    //   - FAQ Document (GEN-009)
                    //   - Blog Article (GEN-005)
                    // Nodes beyond the viewport scroll horizontally.
                </div>
            </div>

            // ── Footer Metadata ────────────────────────────────────────────────
            // Pipeline health and synchronization status.
            // TODO: drive "Last Synced" from the project's last-modified timestamp.
            // "Pipeline Health" should count active/failed/pending jobs.
            // "AI Confidence" is the average VER-004 confidence score across all generated nodes.
            <div class="grid grid-cols-4 gap-8 pt-8 border-t border-slate-200">
                <div class="flex flex-col gap-1">
                    <span class="font-sans text-[10px] font-black uppercase tracking-widest text-slate-400">
                        "Last Synced"
                    </span>
                    <span class="font-serif italic text-primary">"12 Oct 2026, 14:32:01 GMT"</span>
                </div>
                <div class="flex flex-col gap-1">
                    <span class="font-sans text-[10px] font-black uppercase tracking-widest text-slate-400">
                        "Pipeline Health"
                    </span>
                    <div class="flex items-center gap-2">
                        <div class="w-2 h-2 rounded-full bg-emerald-500"></div>
                        <span class="font-serif italic text-primary">"Stable — 2 active workers"</span>
                    </div>
                </div>
                <div class="flex flex-col gap-1">
                    <span class="font-sans text-[10px] font-black uppercase tracking-widest text-slate-400">
                        "AI Confidence"
                    </span>
                    <div class="w-full bg-slate-200 h-1 mt-2">
                        <div class="bg-primary h-1 w-[94%]"></div>
                    </div>
                </div>
                <div class="flex flex-col gap-1 text-right">
                    <span class="font-sans text-[10px] font-black uppercase tracking-widest text-slate-400">
                        "System State"
                    </span>
                    <span class="font-sans font-bold text-emerald-600 uppercase text-[10px]">
                        "Ready for Propagation"
                    </span>
                </div>
            </div>

            // ── Floating Architect's Note (Glassmorphism panel) ────────────────
            // Proactive AI suggestion panel — alerts the user to pipeline inconsistencies.
            // Design: glassmorphism per DESIGN.md §2 "Glass & Gradient Rule":
            //   backdrop-blur-2xl, rgba(255,255,255,0.70), ambient shadow.
            //
            // TODO (CAD-003 + INV-002): "Architect's Note" suggestions are generated by
            // the Inverse Questions engine (Module 9). When a source document is modified,
            // POST /api/pipeline/diff compares the old and new versions (ARI-002 semantic
            // differential) and generates a targeted propagation recommendation.
            <div class="fixed bottom-12 right-12 w-80 bg-white/70 backdrop-blur-2xl p-6 shadow-2xl border border-white/40 rounded-lg z-50">
                <div class="flex items-start gap-4">
                    <div class="p-2 bg-[#C45911]/10 text-[#C45911] rounded">
                        <span class="material-symbols-outlined text-lg">"insights"</span>
                    </div>
                    <div class="flex-1">
                        <h5 class="font-sans font-bold text-xs uppercase tracking-tight text-primary mb-2">
                            "Architect's Note"
                        </h5>
                        <p class="text-sm font-serif text-slate-600 leading-relaxed">
                            "Source document changes detected in "
                            <span class="font-bold text-primary">"Section 4.2"</span>
                            ". Recommended regeneration of the "
                            <span class="italic">"Press Release"</span>
                            " node to maintain integrity."
                        </p>
                        <div class="mt-4 flex gap-3">
                            // TODO: "Accept" → POST /api/pipeline/regenerate-node { node_id: "press_release" }
                            <button class="text-[10px] font-sans font-black uppercase tracking-wider text-primary border-b border-primary pb-0.5">
                                "Accept"
                            </button>
                            // TODO: "Dismiss" → mark this diff notification as dismissed in SQLite
                            <button class="text-[10px] font-sans font-black uppercase tracking-wider text-slate-400 hover:text-red-500 transition-colors">
                                "Dismiss"
                            </button>
                        </div>
                    </div>
                </div>
            </div>
        </section>
    }
}

// Pipeline sub-components

#[component]
fn PipelineSourceNode() -> impl IntoView {
    view! {
        <div class="flex flex-col gap-4">
            <span class="font-sans text-[10px] font-black uppercase tracking-[0.2em] text-slate-400">
                "Root Authority"
            </span>
            <div class="w-72 bg-white p-1 shadow-lg border border-slate-200">
                <div class="bg-surf-cont p-6 border-b border-slate-100 flex flex-col gap-4">
                    <div class="flex justify-between items-start">
                        <div class="bg-primary text-white p-2">
                            <span class="material-symbols-outlined text-lg">"description"</span>
                        </div>
                        // This badge indicates the source has been modified after derivations
                        // TODO (CAD-003): show/hide based on dirty flag from backend
                        <span class="px-2 py-0.5 bg-orange-100 text-[#fa813a] text-[10px] font-bold flex items-center gap-1 border border-[#fa813a]/20">
                            <span class="material-symbols-outlined text-[10px]">"error"</span>
                            "SOURCE UPDATED"
                        </span>
                    </div>
                    <div>
                        <h3 class="font-serif font-bold text-lg text-primary leading-tight">
                            "Institutional Report: Q3 Sovereignty Flux"
                        </h3>
                        <p class="text-[11px] text-slate-400 uppercase mt-1 tracking-wider">
                            "DOC_ID: OLIV-8820-X"
                        </p>
                    </div>
                </div>
                // Document preview (text skeleton + page count)
                <div class="relative h-48 bg-white p-4 overflow-hidden">
                    <div class="space-y-2 opacity-40">
                        <div class="h-2 w-full bg-slate-200"></div>
                        <div class="h-2 w-4/5 bg-slate-200"></div>
                        <div class="h-2 w-full bg-slate-200"></div>
                        <div class="h-2 w-2/3 bg-slate-200"></div>
                        <div class="mt-4 h-20 w-full bg-slate-100"></div>
                    </div>
                    <div class="absolute inset-0 bg-gradient-to-t from-white via-transparent to-transparent"></div>
                    <div class="absolute bottom-4 left-4 right-4 flex justify-between items-center">
                        <span class="text-[10px] font-bold text-slate-400">"142 PAGES"</span>
                        <span class="material-symbols-outlined text-primary text-sm">"open_in_new"</span>
                    </div>
                </div>
            </div>
            <div class="flex items-center gap-2 text-[#C45911] px-1">
                <span class="material-symbols-outlined text-sm">"warning"</span>
                <span class="text-[11px] font-sans font-bold uppercase tracking-tighter">
                    "Propagation Pending"
                </span>
            </div>
        </div>
    }
}

#[component]
fn PipelineDerivedNode(
    layer:        &'static str,
    icon:         &'static str,
    icon_color:   &'static str,
    accent_color: &'static str,
    title:        &'static str,
    desc:         &'static str,
    status:       &'static str,
    status_class: &'static str,
    locked:       bool,
    waiting_for:  &'static str,
) -> impl IntoView {
    let card_class = if locked {
        "w-64 bg-slate-50 border border-slate-200 p-6 flex flex-col gap-4 relative overflow-hidden opacity-80"
    } else {
        "w-64 bg-white border border-slate-200 p-6 shadow-md flex flex-col gap-4 relative overflow-hidden"
    };
    view! {
        <div class="flex flex-col gap-4">
            <span class="font-sans text-[10px] font-black uppercase tracking-[0.2em] text-slate-400">
                {layer}
            </span>
            <div class=card_class>
                // Colored accent strip on right edge — indicates derivation layer
                <div class=format!("absolute top-0 right-0 w-1 h-full {}", accent_color)></div>
                <div class="flex justify-between items-center">
                    <span class=format!("material-symbols-outlined {}", icon_color)>{icon}</span>
                    <span class=format!("px-2 py-0.5 {} text-[10px] font-bold uppercase tracking-wider", status_class)>
                        {status}
                    </span>
                </div>
                <h4 class="font-serif font-bold text-base text-primary">{title}</h4>
                <p class="text-sm font-serif text-slate-500 leading-relaxed italic">{desc}</p>
                <div class="mt-auto pt-4 border-t border-slate-100 flex justify-between items-center">
                    {if locked {
                        view! {
                            <span class="text-[10px] text-slate-400 italic">{waiting_for}</span>
                            <span class="material-symbols-outlined text-slate-300">"lock"</span>
                        }.into_any()
                    } else {
                        view! {
                            // TODO: "Review Now" → navigate to EditorView with this derived
                            // document pre-loaded in the result panel for human editing/approval
                            <button class="text-[10px] font-bold text-blue-600 hover:underline">
                                "Review Now"
                            </button>
                            <span class="material-symbols-outlined text-slate-300 hover:text-primary cursor-pointer">
                                "more_horiz"
                            </span>
                        }.into_any()
                    }}
                </div>
            </div>
        </div>
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// VIEW: AUDIT (Audit Log)
// ═══════════════════════════════════════════════════════════════════════════════
// Tamper-evident operation log for all document processing activities.
// Required for ENS (Esquema Nacional de Seguridad) Category Alta compliance.
//
// Corresponds to SRS §4.4 (Traceability & Audit):
//   TRA-001: log of each transformation (input, prompt, output, user, timestamp)
//   TRA-002: document genealogy (which document derives from which)
//   TRA-003: export audit logs in standard format (JSON-LD or CSV)
//   TRA-004: exportable traceability map (supported assertions → source)
//   SEC-005: audit log of all operations
//
// TODO: fetch log entries from GET /api/audit?project_id=... ordered by timestamp DESC.
// Each entry should show:
//   - Timestamp (ISO 8601)
//   - Operation type (TRANSFORM / ANALYZE / CHAT / EXPORT / PURGE)
//   - Document ID and filename
//   - Transformation module used (e.g. "Module 2 — Executive Summary")
//   - Input hash (SHA-256 of source text)
//   - Output hash (SHA-256 of result text)
//   - Prompt version used
//   - User/session ID (if role-based access is implemented — SEC-003)
//   - Duration (ms)
//
// TODO (SEC-002): audit entries themselves should be hash-chained (each entry
// includes the hash of the previous entry) to detect tampering — similar to a
// blockchain ledger but stored in SQLite.
//
// TODO (TRA-003): "Export Audit Log" button → GET /api/audit/export?format=json|csv
// Returns the full log as a downloadable file.
//
// TODO (PUB-006): run GDPR/LOPDGDD compliance check on export — flag any entries
// that involve personal data (detected via Module 14 — PUB-001) and prompt the
// user to apply redactions before downloading the log.

#[component]
fn AuditView() -> impl IntoView {
    view! {
        <div class="p-10 max-w-7xl mx-auto">
            <header class="mb-10">
                <h2 class="text-4xl font-sans font-black tracking-tighter text-primary uppercase">
                    "Audit Log"
                </h2>
                <p class="font-serif italic text-xl text-outline mt-1">
                    "Tamper-evident operation record — ENS Category Alta"
                </p>
            </header>

            // Placeholder — full implementation in Phase 8
            <div class="bg-white rounded-xl p-16 text-center border border-slate-200/50 shadow-sm">
                <span class="material-symbols-outlined text-[48px] text-primary/20 mb-6 block">
                    "history_edu"
                </span>
                <h3 class="font-sans font-black text-xl text-primary mb-2">
                    "Audit Engine — Phase 8"
                </h3>
                <p class="font-serif italic text-on-surf-var max-w-md mx-auto">
                    "The tamper-evident audit log will record every transformation, analysis,
                    and export operation with SHA-256 hash chaining. Scheduled for Phase 8
                    (ENS Category Alta compliance)."
                </p>
                <div class="mt-8 flex justify-center gap-4">
                    <div class="px-4 py-2 bg-surf-low rounded-sm text-[10px] font-bold uppercase tracking-widest text-outline">
                        "TRA-001 ✓ Designed"
                    </div>
                    <div class="px-4 py-2 bg-surf-low rounded-sm text-[10px] font-bold uppercase tracking-widest text-outline">
                        "SEC-005 ✓ Designed"
                    </div>
                    <div class="px-4 py-2 bg-surf-low rounded-sm text-[10px] font-bold uppercase tracking-widest text-outline">
                        "TRA-003 ✓ Designed"
                    </div>
                </div>
            </div>
        </div>
    }
}
