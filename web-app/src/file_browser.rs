use leptos::prelude::*;
use serde::Deserialize;

// ─── Tipos compartidos ────────────────────────────────────────────────────────

#[derive(Clone, PartialEq, Deserialize, Debug)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub kind: String, // "root" | "dir" | "file"
    pub ext:  String,
    pub size: u64,
}

// ─── Contexto global ─────────────────────────────────────────────────────────
//
// Cualquier componente (chat, plugins vía bridge) puede hacer:
//   let fb = use_context::<FileBrowserCtx>().unwrap();
//   fb.open_with(move |path| { /* usar path */ });

#[derive(Clone, Copy)]
pub struct FileBrowserCtx {
    pub is_open:  RwSignal<bool>,
    pub callback: RwSignal<Option<Callback<String>>>,
}

impl FileBrowserCtx {
    pub fn new() -> Self {
        Self {
            is_open:  RwSignal::new(false),
            callback: RwSignal::new(None),
        }
    }

    /// Abre el explorador. `on_select` se invoca con la ruta absoluta al seleccionar.
    pub fn open_with(&self, on_select: impl Fn(String) + Send + Sync + 'static) {
        self.callback.set(Some(Callback::new(on_select)));
        self.is_open.set(true);
    }
}

// ─── Componente modal ─────────────────────────────────────────────────────────

#[component]
pub fn FileBrowser(ctx: FileBrowserCtx) -> impl IntoView {
    let (current_path, set_current_path) = signal(String::new());
    let (entries, set_entries) = signal::<Vec<DirEntry>>(vec![]);
    let (loading, set_loading) = signal(false);

    // Carga el directorio cuando cambia current_path (o se abre el modal)
    Effect::new(move |_| {
        if !ctx.is_open.get() { return; }
        let path = current_path.get();
        set_loading.set(true);

        wasm_bindgen_futures::spawn_local(async move {
            let url = if path.is_empty() {
                "/browse".to_string()
            } else {
                format!(
                    "/browse?path={}",
                    js_sys::encode_uri_component(&path)
                        .as_string()
                        .unwrap_or_default()
                )
            };

            let result = async {
                let resp = gloo_net::http::Request::get(&url).send().await.ok()?;
                resp.json::<Vec<DirEntry>>().await.ok()
            }.await;

            set_entries.set(result.unwrap_or_default());
            set_loading.set(false);
        });
    });

    // Reset de ruta al abrir
    Effect::new(move |_| {
        if ctx.is_open.get() {
            set_current_path.set(String::new());
        }
    });

    let go_up = move |_| {
        let p = current_path.get();
        if p.is_empty() { return; }
        let parent = std::path::Path::new(&p)
            .parent()
            .map(|pp| pp.to_string_lossy().to_string())
            .unwrap_or_default();
        set_current_path.set(parent);
    };

    let on_entry = move |entry: DirEntry| {
        if entry.kind == "dir" || entry.kind == "root" {
            set_current_path.set(entry.path);
        } else {
            if let Some(cb) = ctx.callback.get() {
                cb.run(entry.path);
            }
            ctx.is_open.set(false);
        }
    };

    view! {
        {move || ctx.is_open.get().then(|| {
            let path_display = {
                let p = current_path.get();
                if p.is_empty() { "Inicio".to_string() } else { p }
            };
            let can_go_up = !current_path.get().is_empty();

            view! {
                // ── Backdrop ──────────────────────────────────────────────────
                <div
                    class="fixed inset-0 bg-black/40 backdrop-blur-sm z-50 flex items-center justify-center"
                    on:click=move |_| ctx.is_open.set(false)
                >
                    // ── Modal ─────────────────────────────────────────────────
                    <div
                        class="bg-white rounded-2xl shadow-2xl w-[560px] max-h-[72vh] \
                               flex flex-col overflow-hidden"
                        on:click=|e| e.stop_propagation()
                    >
                        // Header
                        <div class="flex items-center gap-3 px-5 py-4 border-b border-outline-var/40 shrink-0">
                            <button
                                class=move || format!(
                                    "material-symbols-outlined text-[20px] transition-colors {}",
                                    if can_go_up { "text-on-surf hover:text-primary" }
                                    else { "text-outline-var cursor-default" }
                                )
                                on:click=go_up
                            >
                                "arrow_back"
                            </button>
                            <div class="flex-1 min-w-0">
                                <p class="text-sm font-semibold text-on-surf">"Explorador de archivos"</p>
                                <p class="text-xs text-on-surf-var font-mono truncate">{path_display}</p>
                            </div>
                            <button
                                class="material-symbols-outlined text-[20px] text-on-surf-var hover:text-on-surf"
                                on:click=move |_| ctx.is_open.set(false)
                            >
                                "close"
                            </button>
                        </div>

                        // Contenido
                        <div class="flex-1 overflow-y-auto">
                            {move || {
                                if loading.get() {
                                    view! {
                                        <div class="flex items-center justify-center py-16 text-on-surf-var">
                                            <span class="material-symbols-outlined text-[28px] animate-spin">"progress_activity"</span>
                                        </div>
                                    }.into_any()
                                } else {
                                    let es = entries.get();
                                    if es.is_empty() {
                                        view! {
                                            <div class="flex flex-col items-center justify-center py-16 text-on-surf-var gap-2">
                                                <span class="material-symbols-outlined text-[36px] text-outline-var">"folder_open"</span>
                                                <span class="text-sm">"Carpeta vacía"</span>
                                            </div>
                                        }.into_any()
                                    } else {
                                        es.into_iter().map(|entry| {
                                            let e2   = entry.clone();
                                            let icon = entry_icon(&entry).to_string();
                                            let icol = icon_color(&entry.kind, &entry.ext).to_string();
                                            let name = entry.name.clone();
                                            let sz   = if entry.kind == "file" { format_size(entry.size) } else { String::new() };
                                            let is_dir = entry.kind != "file";
                                            view! {
                                                <button
                                                    class="w-full flex items-center gap-3 px-5 py-3 \
                                                           hover:bg-surf-low transition-colors text-left \
                                                           border-b border-outline-var/20 last:border-0"
                                                    on:click=move |_| on_entry(e2.clone())
                                                >
                                                    <span class=format!("material-symbols-outlined text-[22px] shrink-0 {icol}")>
                                                        {icon}
                                                    </span>
                                                    <span class="flex-1 text-sm text-on-surf truncate">{name}</span>
                                                    {(!sz.is_empty()).then(|| view! {
                                                        <span class="text-xs text-on-surf-var shrink-0">{sz}</span>
                                                    })}
                                                    {is_dir.then(|| view! {
                                                        <span class="material-symbols-outlined text-[16px] text-outline-var shrink-0">
                                                            "chevron_right"
                                                        </span>
                                                    })}
                                                </button>
                                            }
                                        }).collect_view().into_any()
                                    }
                                }
                            }}
                        </div>
                    </div>
                </div>
            }
        })}
    }
}

// ─── Helpers visuales ────────────────────────────────────────────────────────

fn entry_icon(e: &DirEntry) -> &'static str {
    if e.kind != "file" {
        return if e.kind == "root" { "hard_drive" } else { "folder" };
    }
    match e.ext.as_str() {
        "pdf"                                              => "picture_as_pdf",
        "docx" | "doc"                                     => "description",
        "txt" | "md" | "rs" | "py" | "js" | "ts" | "json" => "article",
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp"   => "image",
        "mp4" | "mov" | "avi" | "mkv"                      => "movie",
        "mp3" | "wav" | "ogg" | "flac"                     => "audio_file",
        "zip" | "tar" | "gz" | "7z"                        => "folder_zip",
        _                                                  => "draft",
    }
}

fn icon_color(kind: &str, ext: &str) -> &'static str {
    if kind != "file" { return "text-yellow-500"; }
    match ext {
        "pdf"                                            => "text-red-500",
        "docx" | "doc"                                   => "text-blue-500",
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" => "text-green-500",
        "mp4" | "mov" | "avi"                            => "text-purple-500",
        _                                                => "text-on-surf-var",
    }
}

fn format_size(b: u64) -> String {
    if b < 1_024               { format!("{b} B") }
    else if b < 1_048_576      { format!("{:.1} KB", b as f64 / 1_024.0) }
    else if b < 1_073_741_824  { format!("{:.1} MB", b as f64 / 1_048_576.0) }
    else                       { format!("{:.1} GB", b as f64 / 1_073_741_824.0) }
}
