mod config;
mod db;
mod routes;
mod tools;

use std::sync::{Arc, RwLock};
use std::net::TcpListener as StdTcpListener;

use axum::{Router, routing::get};
use clap::Parser;
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};
use tower_http::cors::{CorsLayer, Any};
use tracing::info;
use wry::WebViewBuilder;

use config::Config;
use routes::{analyse, audit, browse, chat, crisis, download, export, extract, history, plugin_db, plugins, publication, render, settings, transform, transformations, upload, verify};
use routes::settings::Settings;

#[derive(Clone)]
pub struct AppState {
    pub config:   Arc<Config>,
    pub settings: Arc<RwLock<Settings>>,
    pub db:       db::Db,
}

#[derive(Parser, Debug)]
#[command(name = "serve-rs", about = "LocalAiAssistant server")]
pub(crate) struct Cli {
    #[arg(long)] pub port:         Option<u16>,
    #[arg(long)] pub web_dist:     Option<std::path::PathBuf>,
    #[arg(long)] pub plugins_dir:  Option<std::path::PathBuf>,
    #[arg(long)] pub llm_endpoint: Option<String>,
    /// Modo headless: no abre ventana (útil para CI / tests)
    #[arg(long)] pub headless: bool,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "serve_rs=info".into())
        )
        .init();

    let cli      = Cli::parse();
    let headless = cli.headless || std::env::var("HEADLESS").is_ok();
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
    let url  = format!("http://localhost:{}", port);

    // ── Lanzar HTTP server en thread de fondo ─────────────────────────────────
    {
        let config   = config.clone();
        let settings = settings.clone();
        let database = database.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            rt.block_on(async move {
                run_server(config, settings, database, port).await.expect("server error");
            });
        });
    }

    // Dar tiempo al server a arrancar
    std::thread::sleep(std::time::Duration::from_millis(200));

    if headless {
        info!("Modo headless — sin ventana. Ctrl+C para salir.");
        loop { std::thread::sleep(std::time::Duration::from_secs(60)); }
    }

    // ── WebView en el main thread (obligatorio en macOS) ──────────────────────
    let event_loop = EventLoop::new();

    let window = WindowBuilder::new()
        .with_title("LocalAiAssistant")
        .with_inner_size(tao::dpi::LogicalSize::new(1280.0_f64, 820.0_f64))
        .with_min_inner_size(tao::dpi::LogicalSize::new(800.0_f64, 600.0_f64))
        .build(&event_loop)?;

    let _webview = WebViewBuilder::new(&window)
        .with_url(&url)
        .with_devtools(cfg!(debug_assertions))
        .build()?;

    info!("Ventana abierta → {}", url);

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        if let Event::WindowEvent { event: WindowEvent::CloseRequested, .. } = event {
            *control_flow = ControlFlow::Exit;
        }
    });
}

// ── HTTP server ───────────────────────────────────────────────────────────────

async fn run_server(
    config:   Arc<Config>,
    settings: Arc<RwLock<Settings>>,
    database: db::Db,
    port:     u16,
) -> anyhow::Result<()> {
    let state = AppState { config: config.clone(), settings, db: database };

    let app = Router::new()
        .route("/health", get(health))
        .merge(plugin_db::router())
        .merge(chat::router())
        .merge(browse::router())
        .merge(upload::router())
        .merge(download::router())
        .merge(extract::router())
        .merge(analyse::router())
        .merge(audit::router())
        .merge(crisis::router())
        .merge(verify::router())
        .merge(publication::router())
        .merge(transform::router())
        .merge(transformations::router())
        .merge(export::router())
        .merge(render::router())
        .merge(history::router())
        .merge(plugins::router())      // ← plugins ANTES de static_files
        .merge(settings::router())
        .fallback_service(             // ← static_files como fallback GLOBAL al final
            tower_http::services::ServeDir::new(&config.web_dist)
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
