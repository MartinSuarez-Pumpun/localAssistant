use leptos::prelude::*;
use gloo_net::http::Request;
use serde::{Deserialize, Serialize};
use wasm_bindgen_futures;

// ─── Tipos ────────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Default)]
struct LlmSettings {
    llm_endpoint: String,
    llm_model:    String,
    api_key:      String,
}

// ─── Componente ───────────────────────────────────────────────────────────────

#[component]
pub fn ConfigPanel() -> impl IntoView {
    let (endpoint, set_endpoint) = signal(String::new());
    let (model,    set_model)    = signal(String::new());
    let (api_key,  set_api_key)  = signal(String::new());
    let (status,   set_status)   = signal("");

    // Cargar settings actuales al montar
    wasm_bindgen_futures::spawn_local(async move {
        let Ok(resp) = Request::get("/settings").send().await else { return; };
        let Ok(s) = resp.json::<LlmSettings>().await else { return; };
        set_endpoint.set(s.llm_endpoint);
        set_model.set(s.llm_model);
        set_api_key.set(s.api_key);
    });

    let save = move |_| {
        set_status.set("");
        let s = LlmSettings {
            llm_endpoint: endpoint.get(),
            llm_model:    model.get(),
            api_key:      api_key.get(),
        };
        wasm_bindgen_futures::spawn_local(async move {
            let ok = async {
                let resp = Request::post("/settings")
                    .json(&s)
                    .map_err(|e| e.to_string())?
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                Ok::<bool, String>(resp.ok())
            }.await.unwrap_or(false);

            if ok { set_status.set("saved"); }
            else  { set_status.set("error"); }
        });
    };

    view! {
        <div class="flex flex-col h-full bg-surface">
            // Header
            <header class="flex items-center gap-3 px-6 py-4 border-b border-outline-var bg-white/60 backdrop-blur-sm shrink-0">
                <span class="material-symbols-outlined text-primary text-[22px]">"settings"</span>
                <span class="font-semibold text-primary text-sm tracking-tight">"Configuración LLM"</span>
            </header>

            // Form
            <div class="flex-1 overflow-y-auto px-6 py-8">
                <div class="max-w-lg mx-auto space-y-6">

                    <Field
                        label="Endpoint LLM"
                        hint="URL base de la API compatible con OpenAI (ej. http://localhost:11434)"
                        value=Signal::derive(move || endpoint.get())
                        on_input=move |v| set_endpoint.set(v)
                        placeholder="http://localhost:11434"
                    />

                    <Field
                        label="Modelo"
                        hint="Nombre del modelo a usar. Vacío = el servidor decide."
                        value=Signal::derive(move || model.get())
                        on_input=move |v| set_model.set(v)
                        placeholder="llama3, qwen2.5, mistral..."
                    />

                    <Field
                        label="API Key"
                        hint="Opcional. Para servicios remotos (OpenAI, Groq, etc.)."
                        value=Signal::derive(move || api_key.get())
                        on_input=move |v| set_api_key.set(v)
                        placeholder="sk-..."
                        password=true
                    />

                    <div class="flex items-center gap-4 pt-2">
                        <button
                            on:click=save
                            class="px-5 py-2.5 bg-primary text-white rounded-xl font-semibold text-sm \
                                   hover:bg-[#003b65] active:scale-95 transition-all"
                        >
                            "Guardar"
                        </button>

                        {move || match status.get() {
                            "saved" => view! {
                                <span class="text-sm text-green-600 flex items-center gap-1">
                                    <span class="material-symbols-outlined text-[16px]">"check_circle"</span>
                                    "Guardado"
                                </span>
                            }.into_any(),
                            "error" => view! {
                                <span class="text-sm text-red-500 flex items-center gap-1">
                                    <span class="material-symbols-outlined text-[16px]">"error"</span>
                                    "Error al guardar"
                                </span>
                            }.into_any(),
                            _ => view! { <span/> }.into_any(),
                        }}
                    </div>
                </div>
            </div>
        </div>
    }
}

// ─── Campo de formulario ──────────────────────────────────────────────────────

#[component]
fn Field(
    label:    &'static str,
    hint:     &'static str,
    value:    Signal<String>,
    on_input: impl Fn(String) + 'static,
    placeholder: &'static str,
    #[prop(optional)] password: bool,
) -> impl IntoView {
    let input_type = if password { "password" } else { "text" };
    view! {
        <div class="space-y-1">
            <label class="block text-sm font-semibold text-on-surf">{label}</label>
            <input
                type=input_type
                placeholder=placeholder
                class="w-full bg-surf-low border border-outline-var/50 rounded-xl px-4 py-3 \
                       text-sm text-on-surf placeholder-on-surf-var focus:outline-none \
                       focus:ring-1 focus:ring-primary transition-colors"
                prop:value=move || value.get()
                on:input=move |e| on_input(event_target_value(&e))
            />
            <p class="text-xs text-on-surf-var">{hint}</p>
        </div>
    }
}
