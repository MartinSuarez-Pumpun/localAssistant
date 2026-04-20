mod config;
mod db;
mod routes;
mod tools;

use std::sync::{Arc, Mutex, RwLock}; // Mutex usado en KioskLockout vía Arc
use std::net::TcpListener as StdTcpListener;

use axum::{Router, routing::get};
use clap::Parser;
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::WindowBuilder,
};
use tower_http::cors::{CorsLayer, Any};
use tracing::info;
use wry::{WebViewBuilder, WebContext};

use config::Config;
use routes::{autostart, browse, chat, download, export, extract, history, kiosk, plugin_db, plugins, readability, render, settings, transform, upload, workspace};
use routes::settings::Settings;
use routes::kiosk::KioskLockout;

// ─── Evento de usuario para el event loop ─────────────────────────────────────

#[derive(Clone, Debug)]
pub enum UserEvent {
    /// El usuario verificó el PIN correctamente → salir del modo kiosk
    ExitKiosk,
}

// ─── Estado de la aplicación ──────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    pub config:         Arc<Config>,
    pub settings:       Arc<RwLock<Settings>>,
    pub db:             db::Db,
    /// Estado de bloqueo por intentos fallidos de PIN
    pub kiosk_lockout:  Arc<Mutex<KioskLockout>>,
    /// Proxy hacia el event loop nativo (None en modo headless)
    pub event_proxy:    Option<Arc<tao::event_loop::EventLoopProxy<UserEvent>>>,
    /// La app fue iniciada con --kiosk
    pub is_kiosk:       bool,
}

#[derive(Parser, Debug)]
#[command(name = "serve-rs", about = "LocalAiAssistant server")]
pub(crate) struct Cli {
    #[arg(long)] pub port:         Option<u16>,
    #[arg(long)] pub web_dist:     Option<std::path::PathBuf>,
    #[arg(long)] pub plugins_dir:  Option<std::path::PathBuf>,
    #[arg(long)] pub llm_endpoint: Option<String>,
    /// Modo headless: no abre ventana (útil para CI / tests)
    #[arg(long)] pub headless:  bool,
    /// Deshabilitar kiosk: abre en ventana normal en lugar de fullscreen
    #[arg(long)] pub no_kiosk:  bool,
    /// Habilitar inspector WebKit (clic derecho → Inspeccionar)
    #[arg(long)] pub devtools:  bool,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "serve_rs=info".into())
        )
        .init();

    let cli      = Cli::parse();
    let headless = cli.headless || std::env::var("HEADLESS").is_ok();
    // Kiosk es el modo por defecto; --no-kiosk o NO_KIOSK=1 lo desactivan
    let kiosk    = !cli.no_kiosk && std::env::var("NO_KIOSK").is_err();
    let devtools = cli.devtools;
    let config   = Arc::new(Config::from_cli_and_env(&cli));
    let loaded   = Settings::load(&config.settings_path());
    let settings = Arc::new(RwLock::new(loaded));

    // ── Base de datos SQLite (crea ~/.local-ai/ si no existe) ─────────────────
    std::fs::create_dir_all(&*config.ai_base).ok();
    let db_path = config.ai_base.join("oliv.db");
    let database = db::Db::open(&db_path).expect("no se pudo abrir SQLite");
    info!("SQLite         : {}", db_path.display());

    info!("LLM endpoint : {}", settings.read().unwrap().llm_endpoint);
    info!("Web dist     : {}", config.web_dist.display());
    info!("Plugins dir  : {}", config.plugins_dir.display());

    // ── Encontrar puerto libre (síncrono, antes de lanzar nada) ───────────────
    let port = find_free_port(config.port)?;
    let url  = format!("http://127.0.0.1:{}", port);

    // ── En headless no necesitamos event loop ni proxy ────────────────────────
    if headless {
        let app_state = AppState {
            config:        config.clone(),
            settings:      settings.clone(),
            db:            database.clone(),
            kiosk_lockout: Arc::new(Mutex::new(KioskLockout::default())),
            event_proxy:   None,
            is_kiosk:      kiosk,
        };
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            rt.block_on(async move {
                run_server(app_state, port).await.expect("server error");
            });
        });
        wait_for_server(&url);
        info!("Modo headless — sin ventana. Ctrl+C para salir.");
        loop { std::thread::sleep(std::time::Duration::from_secs(60)); }
    }

    // ── WebView en el main thread (obligatorio en macOS) ──────────────────────
    // Crear el event loop ANTES del AppState para poder pasar el proxy
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();

    let proxy = Arc::new(event_loop.create_proxy());

    let app_state = AppState {
        config:        config.clone(),
        settings:      settings.clone(),
        db:            database.clone(),
        kiosk_lockout: Arc::new(Mutex::new(KioskLockout::default())),
        event_proxy:   Some(proxy.clone()),
        is_kiosk:      kiosk,
    };

    // ── Lanzar HTTP server en thread de fondo ─────────────────────────────────
    {
        let state = app_state.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            rt.block_on(async move {
                run_server(state, port).await.expect("server error");
            });
        });
    }

    wait_for_server(&url);

    let mut wb = WindowBuilder::new()
        .with_title("LocalAiAssistant");
    if kiosk {
        wb = wb
            .with_decorations(false)
            .with_fullscreen(Some(tao::window::Fullscreen::Borderless(None)));
    } else {
        wb = wb
            .with_inner_size(tao::dpi::LogicalSize::new(1280.0_f64, 820.0_f64))
            .with_min_inner_size(tao::dpi::LogicalSize::new(800.0_f64, 600.0_f64));
    }
    let window = wb.build(&event_loop)?;

    let data_dir = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("local-ai-webview");
    let mut web_context = WebContext::new(Some(data_dir));

    let ipc_proxy = proxy.clone();
    let webview_builder = WebViewBuilder::with_web_context(&mut web_context)
        .with_url(&url)
        .with_devtools(cfg!(debug_assertions) || devtools)
        .with_initialization_script(r#"
            function _ipc(msg) {
                try {
                    if (window.ipc && typeof window.ipc.postMessage === 'function') {
                        window.ipc.postMessage(msg);
                    }
                } catch(e) {}
            }
            window.onerror = function(msg, src, line, col, err) {
                _ipc('onerror: ' + msg + ' @ ' + src + ':' + line);
            };
            window.addEventListener('unhandledrejection', function(e) {
                _ipc('unhandledrejection: ' + String(e.reason));
            });
            // Forzar repaint tras cargar WASM (ayuda en webkit2gtk/Linux)
            window.addEventListener('TrunkApplicationStarted', function() {
                _ipc('TrunkApplicationStarted');
                var loader = document.getElementById('_loading');
                if (loader) loader.style.display = 'none';
                setTimeout(function() {
                    document.body.style.display = 'none';
                    void document.body.offsetHeight;
                    document.body.style.display = '';
                    _ipc('repaint forced');
                }, 150);
            });
            _ipc('init script loaded');
        "#)
        .with_ipc_handler(move |msg| {
            let body = msg.body();
            eprintln!("[WebView] {}", body);
            // La salida del kiosk puede llegar también por IPC (fallback)
            if body == "exit_kiosk" {
                ipc_proxy.send_event(UserEvent::ExitKiosk).ok();
            }
        });

    // En Linux/Wayland wry no soporta el handle Wayland directamente;
    // hay que construir el WebView sobre el contenedor GTK de tao.
    #[cfg(target_os = "linux")]
    let _webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let vbox = window.default_vbox().unwrap();
        webview_builder.build_gtk(vbox)?
    };
    #[cfg(not(target_os = "linux"))]
    let _webview = webview_builder.build(&window)?;

    info!("Ventana abierta → {}", url);

    // run() retorna `!` — sin `;` actúa como expresión que coerciona a Result<()>
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(UserEvent::ExitKiosk) => {
                // Salir del modo kiosk: restaurar ventana decorada de tamaño normal
                window.set_fullscreen(None);
                window.set_decorations(true);
                window.set_inner_size(tao::dpi::LogicalSize::new(1280.0_f64, 820.0_f64));
                info!("Modo kiosk desactivado por PIN correcto");
            }
            _ => {}
        }
    })
}

// ── HTTP server ───────────────────────────────────────────────────────────────

async fn run_server(state: AppState, port: u16) -> anyhow::Result<()> {
    let web_dist = state.config.web_dist.clone();

    let app = Router::new()
        .route("/health", get(health))
        .merge(autostart::router())
        .merge(kiosk::router())
        .merge(plugin_db::router())
        .merge(chat::router())
        .merge(browse::router())
        .merge(upload::router())
        .merge(download::router())
        .merge(extract::router())
        .merge(transform::router())
        .merge(export::router())
        .merge(render::router())
        .merge(history::router())
        .merge(workspace::router())
        .merge(readability::router())
        .merge(plugins::router())      // ← plugins ANTES de static_files
        .merge(settings::router())
        .fallback_service(             // ← static_files como fallback GLOBAL al final
            tower_http::services::ServeDir::new(&web_dist)
                .append_index_html_on_directories(true),
        )
        .with_state(state)
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        );

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port)).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> &'static str { "ok" }

// ── Espera a que el HTTP server responda /health ──────────────────────────────

fn wait_for_server(base_url: &str) {
    let health = format!("{}/health", base_url);
    for attempt in 0u32..50 {
        let delay = if attempt < 5 { 100 } else { 200 };
        std::thread::sleep(std::time::Duration::from_millis(delay));
        if let Ok(resp) = ureq::get(&health).call() {
            if resp.status() == 200 {
                tracing::info!("Server listo tras {}ms", (attempt as u64 + 1) * delay);
                return;
            }
        }
    }
    tracing::warn!("Server no respondió tras 10s — abriendo WebView de todas formas");
}

// ── Busca puerto libre (síncrono) ─────────────────────────────────────────────

fn find_free_port(preferred: u16) -> anyhow::Result<u16> {
    for offset in 0u16..20 {
        let port = preferred.saturating_add(offset);
        if StdTcpListener::bind(format!("127.0.0.1:{}", port)).is_ok() {
            return Ok(port);
        }
        tracing::warn!("puerto {} ocupado, probando {}...", port, port + 1);
    }
    anyhow::bail!("no hay puerto libre entre {} y {}", preferred, preferred + 19)
}
