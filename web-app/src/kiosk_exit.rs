/// Zona de escape del modo kiosk.
///
/// Implementación:
///   - Div invisible 60×60 px en la esquina inferior-izquierda.
///   - 5 clicks en menos de 3 segundos abre el modal PIN.
///   - El PIN se verifica contra el hash argon2 en el servidor.
///   - Bloqueo exponencial tras N intentos fallidos (gestionado en servidor).
///   - En éxito el servidor envía UserEvent::ExitKiosk al event loop nativo.
use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsCast as _;

// ─── DTOs ─────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct VerifyBody {
    pin: String,
}

#[derive(Deserialize)]
struct VerifyResp {
    ok:            bool,
    locked_until:  Option<f64>, // unix timestamp segundos
    attempts_left: Option<u32>,
    error:         Option<String>,
}

#[derive(Deserialize)]
struct StatusResp {
    has_pin:  bool,
    is_kiosk: bool,
}

// ─── Componente ───────────────────────────────────────────────────────────────

#[component]
pub fn KioskExit() -> impl IntoView {
    // Estado del contador de clicks
    let (click_count,     set_click_count)     = signal(0u32);
    let (last_click_ms,   set_last_click_ms)   = signal(0f64);
    // Modal
    let (show_modal,      set_show_modal)      = signal(false);
    let (pin,             set_pin)             = signal(String::new());
    let (error_msg,       set_error_msg)       = signal(Option::<String>::None);
    let (locked_until_ms, set_locked_until_ms) = signal(Option::<f64>::None);
    let (loading,         set_loading)         = signal(false);
    // La zona activa se muestra siempre en modo kiosk (tenga PIN o no)
    let (kiosk_active, set_kiosk_active) = signal(false);
    let (has_pin,      set_has_pin)      = signal(false);

    // Comprobar el estado al montar
    wasm_bindgen_futures::spawn_local(async move {
        if let Ok(resp) = gloo_net::http::Request::get("/api/kiosk/status").send().await {
            if let Ok(s) = resp.json::<StatusResp>().await {
                set_kiosk_active.set(s.is_kiosk);
                set_has_pin.set(s.has_pin);
            }
        }
    });

    // ── Lógica de clicks en la zona oculta ────────────────────────────────────
    let on_hotzone_click = move |_| {
        let now  = js_sys::Date::now(); // milisegundos
        let last = last_click_ms.get();

        let count = if now - last > 3_000.0 {
            1 // ventana expirada → reiniciar
        } else {
            click_count.get() + 1
        };

        set_last_click_ms.set(now);
        set_click_count.set(count);

        if count >= 5 {
            set_click_count.set(0);
            set_pin.set(String::new());
            set_error_msg.set(None);
            set_locked_until_ms.set(None);
            set_show_modal.set(true);
        }
    };

    // ── Envío del PIN ─────────────────────────────────────────────────────────
    let do_submit = move || {
        let pin_val = pin.get();
        if pin_val.is_empty() || loading.get() {
            return;
        }
        set_loading.set(true);
        set_error_msg.set(None);

        wasm_bindgen_futures::spawn_local(async move {
            let body = serde_json::to_string(&VerifyBody { pin: pin_val }).unwrap();
            let result = gloo_net::http::Request::post("/api/kiosk/verify")
                .header("Content-Type", "application/json")
                .body(body)
                .unwrap()
                .send()
                .await;

            set_loading.set(false);

            match result {
                Err(_) => set_error_msg.set(Some("Error de conexión".into())),
                Ok(resp) => {
                    match resp.json::<VerifyResp>().await {
                        Err(_) => set_error_msg.set(Some("Respuesta inesperada".into())),
                        Ok(body) => {
                            if body.ok {
                                set_show_modal.set(false);
                                // El servidor ya envió UserEvent::ExitKiosk al event loop.
                                // Como fallback, también lo enviamos por IPC desde el WASM.
                                send_ipc_exit_kiosk();
                            } else {
                                if let Some(lu) = body.locked_until {
                                    set_locked_until_ms.set(Some(lu * 1_000.0));
                                }
                                set_error_msg.set(body.error.or(Some("PIN incorrecto".into())));
                            }
                        }
                    }
                }
            }
        });
    };

    // ─── Vista ────────────────────────────────────────────────────────────────
    view! {
        // Zona activa oculta — esquina inferior izquierda, 60×60 px
        // Se monta siempre en kiosk (tenga PIN o no — el modal explica si falta)
        {move || kiosk_active.get().then(|| view! {
            <div
                style="position:fixed;bottom:0;right:0;\
                       width:60px;height:60px;\
                       z-index:9998;\
                       cursor:default;\
                       user-select:none;\
                       -webkit-user-select:none"
                on:click=on_hotzone_click
            />
        })}

        // Modal PIN
        {move || show_modal.get().then(|| {
            let do_submit2 = do_submit;
            let no_pin     = !has_pin.get();
            view! {
                <div
                    style="position:fixed;inset:0;\
                           z-index:10000;\
                           display:flex;\
                           align-items:center;\
                           justify-content:center;\
                           background:rgba(0,0,0,0.72);\
                           backdrop-filter:blur(6px)"
                >
                    <div
                        style="background:#1e1e2e;\
                               border-radius:16px;\
                               padding:32px;\
                               max-width:320px;\
                               width:calc(100% - 32px);\
                               box-shadow:0 25px 50px rgba(0,0,0,0.6);\
                               border:1px solid rgba(255,255,255,0.1);\
                               display:flex;\
                               flex-direction:column;\
                               gap:12px"
                        on:click=|e| e.stop_propagation()
                    >
                        // ── Sin PIN configurado ─────────────────────────────
                        {no_pin.then(|| view! {
                            <p style="margin:0;color:#fbbf24;font-size:0.88rem;line-height:1.5;\
                                      background:#78350f33;border-radius:8px;padding:10px 12px;\
                                      border:1px solid #78350f88">
                                "⚠ No hay PIN configurado. Establece uno con:"
                                <br/>
                                <code style="font-size:0.78rem;color:#fde68a;user-select:text">
                                    "POST /api/kiosk/pin  {\"pin\":\"1234\"}"
                                </code>
                            </p>
                        })}
                        <div style="display:flex;align-items:center;gap:10px">
                            <span
                                class="material-symbols-outlined"
                                style="color:#60a5fa;font-size:24px"
                            >
                                "lock_open"
                            </span>
                            <h2 style="margin:0;color:#f1f5f9;\
                                       font-size:1.05rem;font-weight:600">
                                "Salir del modo kiosko"
                            </h2>
                        </div>

                        <p style="margin:0;color:#94a3b8;font-size:0.83rem;line-height:1.5">
                            "Introduce el PIN de administrador para restaurar la ventana."
                        </p>

                        // Campo PIN
                        <input
                            type="password"
                            inputmode="numeric"
                            maxlength="20"
                            placeholder="PIN"
                            autofocus=true
                            prop:value=move || pin.get()
                            on:input=move |e| {
                                if let Some(el) = e.target()
                                    .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
                                {
                                    set_pin.set(el.value());
                                }
                            }
                            on:keydown=move |e: web_sys::KeyboardEvent| {
                                match e.key().as_str() {
                                    "Enter"  => do_submit(),
                                    "Escape" => set_show_modal.set(false),
                                    _        => {}
                                }
                            }
                            style="padding:10px 14px;\
                                   border-radius:8px;\
                                   background:#0f172a;\
                                   border:1px solid rgba(255,255,255,0.15);\
                                   color:#f1f5f9;\
                                   font-size:1.25rem;\
                                   letter-spacing:0.35em;\
                                   outline:none;\
                                   width:100%;\
                                   box-sizing:border-box"
                        />

                        // Mensaje de error / bloqueo
                        {move || {
                            let err = error_msg.get();
                            let lu  = locked_until_ms.get();
                            err.map(|msg| view! {
                                <p style="margin:0;color:#f87171;font-size:0.8rem;line-height:1.4">
                                    {msg}
                                    {lu.map(|ts| {
                                        let secs = ((ts - js_sys::Date::now()) / 1_000.0).ceil() as i64;
                                        if secs > 0 {
                                            let label = if secs < 60 {
                                                format!(" ({} s)", secs)
                                            } else {
                                                format!(" ({} min)", (secs as f64 / 60.0).ceil() as i64)
                                            };
                                            Some(view! { <span style="color:#fca5a5">{label}</span> })
                                        } else {
                                            None
                                        }
                                    })}
                                </p>
                            })
                        }}

                        // Botones
                        <div style="display:flex;gap:8px;justify-content:flex-end;margin-top:4px">
                            <button
                                on:click=move |_| set_show_modal.set(false)
                                style="padding:8px 18px;\
                                       border-radius:8px;\
                                       background:transparent;\
                                       border:1px solid rgba(255,255,255,0.15);\
                                       color:#94a3b8;\
                                       cursor:pointer;\
                                       font-size:0.88rem"
                            >
                                "Cancelar"
                            </button>
                            <button
                                on:click=move |_| do_submit2()
                                disabled=move || loading.get()
                                style="padding:8px 18px;\
                                       border-radius:8px;\
                                       background:#1a56db;\
                                       border:none;\
                                       color:#fff;\
                                       cursor:pointer;\
                                       font-size:0.88rem;\
                                       font-weight:500;\
                                       opacity: 1"
                            >
                                {move || if loading.get() { "Verificando…" } else { "Confirmar" }}
                            </button>
                        </div>
                    </div>
                </div>
            }
        })}
    }
}

// ─── IPC fallback ─────────────────────────────────────────────────────────────

/// Intenta enviar "exit_kiosk" por wry IPC (window.ipc.postMessage).
/// El servidor lo recibe en el ipc_handler y emite UserEvent::ExitKiosk.
fn send_ipc_exit_kiosk() {
    let _ = (|| -> Option<()> {
        let win = web_sys::window()?;
        let ipc = js_sys::Reflect::get(&win, &wasm_bindgen::JsValue::from_str("ipc")).ok()?;
        let post_fn = js_sys::Reflect::get(&ipc, &wasm_bindgen::JsValue::from_str("postMessage"))
            .ok()?
            .dyn_into::<js_sys::Function>()
            .ok()?;
        post_fn
            .call1(&ipc, &wasm_bindgen::JsValue::from_str("exit_kiosk"))
            .ok()?;
        Some(())
    })();
}
