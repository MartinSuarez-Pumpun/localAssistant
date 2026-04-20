use leptos::prelude::*;
use gloo_net::http::Request;
use serde::Deserialize;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::chat::ChatPanel;
use crate::config_panel::ConfigPanel;
use crate::file_browser::{FileBrowser, FileBrowserCtx};
use crate::kiosk_exit::KioskExit;
use crate::plugins::PluginInfo;

// ─── Tipos autostart ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct AutostartStatus {
    enabled: bool,
    asked:   bool,
}

// ─── Vista activa ─────────────────────────────────────────────────────────────

#[derive(Clone, PartialEq)]
enum PluginsState {
    Loading,
    Ready(Vec<PluginInfo>),
}

#[derive(Clone, PartialEq)]
enum ActiveView {
    Chat,
    Config,
    Plugin(String),
}

fn has_no_plugins(state: &PluginsState) -> bool {
    match state {
        PluginsState::Ready(ps) => ps.is_empty(),
        PluginsState::Loading   => true,
    }
}

// ─── Shell principal ──────────────────────────────────────────────────────────

#[component]
pub fn Shell() -> impl IntoView {
    let (active, set_active)         = signal(ActiveView::Chat);
    let (plugins, set_plugins)       = signal(PluginsState::Loading);
    let (sidebar_bg, set_sidebar_bg) = signal(String::new()); // vacío = color base
    let (show_autostart, set_show_autostart) = signal(false);

    // ── Contexto global del explorador de archivos ────────────────────────────
    let fb_ctx = FileBrowserCtx::new();
    provide_context(fb_ctx);

    // ── Plugin bridge: escuchar postMessage de iframes ────────────────────────
    //
    // Los plugins pueden abrir el explorador con:
    //   window.parent.postMessage({ type: "localai:open_files", id: "req-1" }, "*")
    // Y recibirán la selección:
    //   { type: "localai:file_selected", id: "req-1", path: "/..." }
    {
        let fb_ctx = fb_ctx;
        let closure = Closure::<dyn Fn(web_sys::MessageEvent)>::new(
            move |e: web_sys::MessageEvent| {
                let data = e.data();
                let msg_type = js_sys::Reflect::get(&data, &JsValue::from_str("type"))
                    .ok()
                    .and_then(|v| v.as_string());

                // ── localai:sidebar_color → el plugin define el color de la sidebar ──
                if msg_type.as_deref() == Some("localai:sidebar_color") {
                    let color = js_sys::Reflect::get(&data, &JsValue::from_str("color"))
                        .ok()
                        .and_then(|v| v.as_string())
                        .unwrap_or_default();
                    set_sidebar_bg.set(color);
                    return;
                }

                if msg_type.as_deref() != Some("localai:open_files") { return; }

                let req_id = js_sys::Reflect::get(&data, &JsValue::from_str("id"))
                    .ok()
                    .and_then(|v| v.as_string())
                    .unwrap_or_default();

                // Guardar la fuente (ventana del plugin) como JsValue
                let source: JsValue = match e.source() {
                    Some(s) => s.into(),
                    None    => return,
                };

                fb_ctx.open_with(move |path: String| {
                    // Construir mensaje de respuesta vía eval (evita complejidad de web-sys)
                    let safe_path = path.replace('\'', "\\'").replace('\\', "\\\\");
                    let safe_id   = req_id.replace('\'', "\\'");
                    let code = format!(
                        "(function(w){{ w.postMessage({{type:'localai:file_selected',id:'{safe_id}',path:'{safe_path}'}}, '*') }})(arguments[0])"
                    );
                    let fn_val = js_sys::Function::new_with_args("arguments", &code);
                    fn_val.call1(&JsValue::NULL, &source).ok();
                });
            }
        );

        web_sys::window()
            .unwrap()
            .add_event_listener_with_callback("message", closure.as_ref().unchecked_ref())
            .ok();
        closure.forget(); // listener global — no se elimina nunca
    }

    // ── Cargar plugins ────────────────────────────────────────────────────────
    wasm_bindgen_futures::spawn_local(async move {
        let result = async {
            let resp = Request::get("/plugins/").send().await.ok()?;
            resp.json::<Vec<PluginInfo>>().await.ok()
        }.await;
        set_plugins.set(PluginsState::Ready(result.unwrap_or_default()));
    });

    // ── Comprobar autostart (mostrar modal solo si no se preguntó antes) ──────
    wasm_bindgen_futures::spawn_local(async move {
        if let Ok(resp) = Request::get("/api/autostart").send().await {
            if let Ok(status) = resp.json::<AutostartStatus>().await {
                if !status.enabled && !status.asked {
                    set_show_autostart.set(true);
                }
            }
        }
    });

    view! {
        <div class="flex h-screen w-screen overflow-hidden bg-surface font-sans text-on-surf">

            // ── Sidebar ───────────────────────────────────────────────────────
            {move || {
                match plugins.get() {
                    PluginsState::Loading => ().into_any(),
                    PluginsState::Ready(ps) if ps.is_empty() => ().into_any(),
                    PluginsState::Ready(ps) => view! {
                        <nav
                            class="relative w-14 shrink-0 overflow-hidden"
                            style="background-color:#1a56db"
                        >
                            // ── Overlay top-down: el plugin pinta su color ────────
                            <div
                                class="absolute inset-0 pointer-events-none z-0"
                                style=move || {
                                    let c = sidebar_bg.get();
                                    if c.is_empty() {
                                        "transform-origin:top;\
                                         transform:scaleY(0);\
                                         opacity:0;\
                                         transition:opacity 0.4s ease-in,\
                                                     transform 0s 0.4s linear;\
                                         background-color:transparent".to_string()
                                    } else {
                                        format!(
                                            "transform-origin:top;\
                                             transform:scaleY(1);\
                                             opacity:1;\
                                             transition:transform 0.55s cubic-bezier(0.4,0,0.2,1),\
                                                         opacity 0.55s ease-out;\
                                             background-color:{c}"
                                        )
                                    }
                                }
                            />

                            // ── Contenido de la sidebar (encima del overlay) ──────
                            <div class="relative z-10 flex flex-col w-full h-full \
                                        items-center py-4 gap-1"
                                 style="border-right:1px solid #ffffff3b"
                            >

                                <div class="mb-4 select-none" title="LocalAiAssistant">
                                    <span class="material-symbols-outlined text-white/40 text-[20px]">"hub"</span>
                                </div>

                                <NavBtn
                                    icon="chat".to_string()
                                    label="Chat".to_string()
                                    active=Signal::derive(move || active.get() == ActiveView::Chat)
                                    on_click=move || {
                                        set_active.set(ActiveView::Chat);
                                        set_sidebar_bg.set(String::new());
                                    }
                                />

                                <div class="w-6 h-px bg-white/10 my-1"/>

                                {ps.into_iter().map(|p| {
                                    let id1 = p.id.clone();
                                    let id2 = p.id.clone();
                                    view! {
                                        <NavBtn
                                            icon=p.icon.clone()
                                            label=p.name.clone()
                                            active=Signal::derive(move || active.get() == ActiveView::Plugin(id1.clone()))
                                            on_click=move || set_active.set(ActiveView::Plugin(id2.clone()))
                                        />
                                    }
                                }).collect_view()}

                                <div class="flex-1"/>

                                <NavBtn
                                    icon="settings".to_string()
                                    label="Configuración LLM".to_string()
                                    active=Signal::derive(move || active.get() == ActiveView::Config)
                                    on_click=move || {
                                        set_active.set(ActiveView::Config);
                                        set_sidebar_bg.set(String::new());
                                    }
                                />
                            </div>
                        </nav>
                    }.into_any(),
                }
            }}

            // ── Contenido principal ───────────────────────────────────────────
            <main class="flex-1 overflow-hidden relative">
                {move || match active.get() {
                    ActiveView::Chat => view! {
                        <div class="h-full overflow-hidden">
                            <ChatPanel/>
                            {move || has_no_plugins(&plugins.get()).then(|| view! {
                                <button
                                    title="Configuración LLM"
                                    on:click=move |_| set_active.set(ActiveView::Config)
                                    class="absolute top-3 right-4 w-9 h-9 rounded-lg flex items-center \
                                           justify-center text-on-surf-var hover:bg-surf-cont \
                                           transition-colors material-symbols-outlined text-[20px]"
                                >
                                    "settings"
                                </button>
                            })}
                        </div>
                    }.into_any(),

                    ActiveView::Config => view! {
                        <div class="h-full overflow-hidden">
                            <ConfigPanel/>
                            {move || has_no_plugins(&plugins.get()).then(|| view! {
                                <button
                                    title="Volver al chat"
                                    on:click=move |_| set_active.set(ActiveView::Chat)
                                    class="absolute top-3 right-4 w-9 h-9 rounded-lg flex items-center \
                                           justify-center text-on-surf-var hover:bg-surf-cont \
                                           transition-colors material-symbols-outlined text-[20px]"
                                >
                                    "close"
                                </button>
                            })}
                        </div>
                    }.into_any(),

                    ActiveView::Plugin(id) => {
                        let src = format!("/plugins/{}/app/", id);
                        view! {
                            <iframe src=src class="w-full h-full border-0 block"></iframe>
                        }.into_any()
                    }
                }}
            </main>
        </div>

        // ── Modal explorador de archivos (global, sobre todo) ─────────────────
        <FileBrowser ctx=fb_ctx/>

        // ── Zona de escape del modo kiosk (invisible, esquina inferior-izquierda)
        <KioskExit/>

        // ── Modal autostart ───────────────────────────────────────────────────
        {move || show_autostart.get().then(|| view! {
            <div class="fixed inset-0 z-50 flex items-center justify-center \
                        bg-black/60 backdrop-blur-sm">
                <div class="bg-surf-cont rounded-2xl shadow-2xl p-8 max-w-sm w-full mx-4 \
                            flex flex-col gap-5 border border-white/10">
                    <div class="flex items-center gap-3">
                        <span class="material-symbols-outlined text-primary text-[28px]">
                            "rocket_launch"
                        </span>
                        <h2 class="text-on-surf font-semibold text-lg leading-tight">
                            "Abrir al iniciar el sistema"
                        </h2>
                    </div>
                    <p class="text-on-surf-var text-sm leading-relaxed">
                        "¿Quieres que LocalAI Assistant se abra automáticamente \
                         cada vez que enciendas el equipo?"
                    </p>
                    <div class="flex gap-3 justify-end mt-1">
                        <button
                            on:click=move |_| {
                                set_show_autostart.set(false);
                                wasm_bindgen_futures::spawn_local(async move {
                                    let _ = Request::post("/api/autostart")
                                        .header("Content-Type", "application/json")
                                        .body("{\"enabled\":false}")
                                        .unwrap()
                                        .send()
                                        .await;
                                });
                            }
                            class="px-4 py-2 rounded-lg text-sm text-on-surf-var \
                                   hover:bg-white/8 transition-colors"
                        >
                            "Ahora no"
                        </button>
                        <button
                            on:click=move |_| {
                                set_show_autostart.set(false);
                                wasm_bindgen_futures::spawn_local(async move {
                                    let _ = Request::post("/api/autostart")
                                        .header("Content-Type", "application/json")
                                        .body("{\"enabled\":true}")
                                        .unwrap()
                                        .send()
                                        .await;
                                });
                            }
                            class="px-4 py-2 rounded-lg text-sm font-medium \
                                   bg-primary text-white hover:bg-primary/90 \
                                   transition-colors"
                        >
                            "Sí, configurar"
                        </button>
                    </div>
                </div>
            </div>
        })}
    }
}

// ─── Botón de navegación ──────────────────────────────────────────────────────

#[component]
fn NavBtn(
    icon:     String,
    label:    String,
    active:   Signal<bool>,
    on_click: impl Fn() + 'static,
) -> impl IntoView {
    let cls = move || format!(
        "w-10 h-10 rounded flex items-center justify-center transition-colors \
         material-symbols-outlined text-[20px] {}",
        if active.get() {
            "bg-white/15 text-primary-cont"
        } else {
            "text-white/40 hover:bg-white/8 hover:text-white/70"
        }
    );
    view! {
        <button title=label on:click=move |_| on_click() class=cls>
            {icon}
        </button>
    }
}
