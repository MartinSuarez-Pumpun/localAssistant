// Modal explorador de archivos para el plugin — portado del web-app.
// El iframe comparte origen con el host (localhost:PORT) así que puede
// llamar directamente al endpoint /browse del servidor sin postMessage.
//
// Uso:
//   let fb = use_context::<FileBrowserCtx>().unwrap();
//   fb.open_with(move |path| { /* cargar archivo por ruta */ });

use leptos::prelude::*;
use serde::Deserialize;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{spawn_local, JsFuture};

#[derive(Clone, PartialEq, Deserialize, Debug)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub kind: String, // "root" | "dir" | "file"
    pub ext:  String,
    pub size: u64,
}

#[derive(Clone, Copy, PartialEq)]
pub enum PickMode {
    File, // selecciona archivo → callback(path_archivo)
    Dir,  // selecciona carpeta → callback(path_carpeta); el header muestra "Guardar aquí"
}

#[derive(Clone, Copy)]
pub struct FileBrowserCtx {
    pub is_open:   RwSignal<bool>,
    pub callback:  RwSignal<Option<Callback<String>>>,
    pub accept:    RwSignal<Option<Vec<String>>>, // extensiones permitidas, None = todas
    pub mode:      RwSignal<PickMode>,
    pub hint_name: RwSignal<String>, // preview del nombre en modo Dir
}

impl FileBrowserCtx {
    pub fn new() -> Self {
        Self {
            is_open:   RwSignal::new(false),
            callback:  RwSignal::new(None),
            accept:    RwSignal::new(None),
            mode:      RwSignal::new(PickMode::File),
            hint_name: RwSignal::new(String::new()),
        }
    }

    pub fn open_with(&self, on_select: impl Fn(String) + Send + Sync + 'static) {
        self.callback.set(Some(Callback::new(on_select)));
        self.accept.set(None);
        self.mode.set(PickMode::File);
        self.hint_name.set(String::new());
        self.is_open.set(true);
    }

    pub fn open_with_filter(
        &self,
        exts: Vec<&str>,
        on_select: impl Fn(String) + Send + Sync + 'static,
    ) {
        self.callback.set(Some(Callback::new(on_select)));
        self.accept.set(Some(exts.into_iter().map(|s| s.to_string()).collect()));
        self.mode.set(PickMode::File);
        self.hint_name.set(String::new());
        self.is_open.set(true);
    }

    /// Abre el modal para elegir un directorio destino. `filename_hint` se
    /// muestra en el header como referencia ("se guardará {hint} aquí") —
    /// el servidor es quien escribe el archivo final.
    pub fn pick_dir(
        &self,
        filename_hint: impl Into<String>,
        on_select: impl Fn(String) + Send + Sync + 'static,
    ) {
        self.callback.set(Some(Callback::new(on_select)));
        self.accept.set(None);
        self.mode.set(PickMode::Dir);
        self.hint_name.set(filename_hint.into());
        self.is_open.set(true);
    }
}

async fn fetch_dir(path: &str) -> Vec<DirEntry> {
    let url = if path.is_empty() {
        "/browse".to_string()
    } else {
        format!("/browse?path={}",
            js_sys::encode_uri_component(path).as_string().unwrap_or_default())
    };
    let Some(window) = web_sys::window() else { return vec![] };
    let req = match web_sys::Request::new_with_str(&url) {
        Ok(r)  => r,
        Err(_) => return vec![],
    };
    let resp: web_sys::Response = match JsFuture::from(window.fetch_with_request(&req)).await {
        Ok(v)  => v.unchecked_into(),
        Err(_) => return vec![],
    };
    if !resp.ok() { return vec![]; }
    let Ok(text_promise) = resp.text() else { return vec![] };
    let Ok(js_text) = JsFuture::from(text_promise).await else { return vec![] };
    let s = js_text.as_string().unwrap_or_default();
    serde_json::from_str::<Vec<DirEntry>>(&s).unwrap_or_default()
}

#[component]
pub fn FileBrowser(ctx: FileBrowserCtx) -> impl IntoView {
    let (current_path, set_current_path) = signal(String::new());
    let (entries, set_entries) = signal::<Vec<DirEntry>>(vec![]);
    let (loading, set_loading) = signal(false);

    Effect::new(move |_| {
        if !ctx.is_open.get() { return; }
        let path = current_path.get();
        set_loading.set(true);
        spawn_local(async move {
            let result = fetch_dir(&path).await;
            set_entries.set(result);
            set_loading.set(false);
        });
    });

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

    let is_accepted = move |ext: &str| -> bool {
        match ctx.accept.get() {
            None => true,
            Some(allowed) => allowed.iter().any(|e| e == ext),
        }
    };

    let on_entry = move |entry: DirEntry| {
        // En modo Dir sólo se navega; la selección se confirma con el botón
        // "Guardar aquí". Los archivos se muestran pero no son clicables.
        if entry.kind == "dir" || entry.kind == "root" {
            set_current_path.set(entry.path);
        } else if ctx.mode.get() == PickMode::File && is_accepted(&entry.ext) {
            if let Some(cb) = ctx.callback.get() {
                cb.run(entry.path);
            }
            ctx.is_open.set(false);
        }
    };

    let confirm_dir = move |_| {
        let p = current_path.get();
        if p.is_empty() { return; }
        if let Some(cb) = ctx.callback.get() {
            cb.run(p);
        }
        ctx.is_open.set(false);
    };

    view! {
        {move || ctx.is_open.get().then(|| {
            let path_display = {
                let p = current_path.get();
                if p.is_empty() { "Inicio".to_string() } else { p }
            };
            let can_go_up = !current_path.get().is_empty();
            let accept_label = ctx.accept.get().map(|v| format!(".{}", v.join(", ."))).unwrap_or_default();
            let is_pick_dir = ctx.mode.get() == PickMode::Dir;
            let hint_name   = ctx.hint_name.get();
            let can_save    = !current_path.get().is_empty();

            view! {
                <div
                    class="fixed inset-0 bg-black/40 backdrop-blur-sm z-[100] flex items-center justify-center"
                    on:click=move |_| ctx.is_open.set(false)
                >
                    <div
                        class="bg-white rounded-2xl shadow-2xl w-[560px] max-h-[72vh] \
                               flex flex-col overflow-hidden"
                        on:click=|e| e.stop_propagation()
                    >
                        <div class="flex items-center gap-3 px-5 py-4 border-b border-slate-200 shrink-0">
                            <button
                                class=move || format!(
                                    "material-symbols-outlined text-[20px] transition-colors {}",
                                    if can_go_up { "text-slate-700 hover:text-primary" }
                                    else { "text-slate-300 cursor-default" }
                                )
                                on:click=go_up
                            >
                                "arrow_back"
                            </button>
                            <div class="flex-1 min-w-0">
                                <p class="text-sm font-semibold text-slate-800">
                                    {if is_pick_dir { "Elegir carpeta destino" } else { "Explorador de archivos" }}
                                </p>
                                <p class="text-xs text-slate-500 font-mono truncate">{path_display}</p>
                                {(!accept_label.is_empty() && !is_pick_dir).then(|| view! {
                                    <p class="text-[10px] text-slate-400 mt-0.5">"Filtrando: " {accept_label}</p>
                                })}
                                {(is_pick_dir && !hint_name.is_empty()).then(|| {
                                    let h = hint_name.clone();
                                    view! {
                                        <p class="text-[10px] text-slate-400 mt-0.5 truncate">
                                            "Se guardará como: " <span class="font-mono">{h}</span>
                                        </p>
                                    }
                                })}
                            </div>
                            {is_pick_dir.then(|| view! {
                                <button
                                    class=move || format!(
                                        "flex items-center gap-1 px-3 py-1.5 rounded-lg text-[11px] font-bold uppercase tracking-wider transition-colors {}",
                                        if can_save { "bg-primary text-white hover:bg-[#003b65]" }
                                        else        { "bg-slate-200 text-slate-400 cursor-not-allowed" }
                                    )
                                    disabled=!can_save
                                    on:click=confirm_dir
                                >
                                    <span class="material-symbols-outlined text-[16px]">"save"</span>
                                    "Guardar aquí"
                                </button>
                            })}
                            <button
                                class="material-symbols-outlined text-[20px] text-slate-500 hover:text-slate-800"
                                on:click=move |_| ctx.is_open.set(false)
                            >
                                "close"
                            </button>
                        </div>

                        <div class="flex-1 overflow-y-auto">
                            {move || {
                                if loading.get() {
                                    view! {
                                        <div class="flex items-center justify-center py-16 text-slate-500">
                                            <span class="material-symbols-outlined text-[28px] animate-spin">"progress_activity"</span>
                                        </div>
                                    }.into_any()
                                } else {
                                    let es = entries.get();
                                    if es.is_empty() {
                                        view! {
                                            <div class="flex flex-col items-center justify-center py-16 text-slate-500 gap-2">
                                                <span class="material-symbols-outlined text-[36px] text-slate-300">"folder_open"</span>
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
                                            let disabled = entry.kind == "file" && !is_accepted(&entry.ext);
                                            view! {
                                                <button
                                                    class=format!(
                                                        "w-full flex items-center gap-3 px-5 py-3 text-left \
                                                         border-b border-slate-100 last:border-0 transition-colors {}",
                                                        if disabled { "opacity-40 cursor-not-allowed" }
                                                        else { "hover:bg-slate-50" }
                                                    )
                                                    disabled=disabled
                                                    on:click=move |_| on_entry(e2.clone())
                                                >
                                                    <span class=format!("material-symbols-outlined text-[22px] shrink-0 {icol}")>
                                                        {icon}
                                                    </span>
                                                    <span class="flex-1 text-sm text-slate-800 truncate">{name}</span>
                                                    {(!sz.is_empty()).then(|| view! {
                                                        <span class="text-xs text-slate-500 shrink-0">{sz}</span>
                                                    })}
                                                    {is_dir.then(|| view! {
                                                        <span class="material-symbols-outlined text-[16px] text-slate-300 shrink-0">
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
        _                                                => "text-slate-500",
    }
}

fn format_size(b: u64) -> String {
    if b < 1_024               { format!("{b} B") }
    else if b < 1_048_576      { format!("{:.1} KB", b as f64 / 1_024.0) }
    else if b < 1_073_741_824  { format!("{:.1} MB", b as f64 / 1_048_576.0) }
    else                       { format!("{:.1} GB", b as f64 / 1_073_741_824.0) }
}
