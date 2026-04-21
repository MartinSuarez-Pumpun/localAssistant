use std::path::PathBuf;
use crate::Cli;

pub struct Config {
    pub port: u16,
    pub llm_endpoint: String,
    pub llm_model: String,
    pub web_dist: PathBuf,
    pub plugins_dir: PathBuf,
    pub ai_base: PathBuf,
}

impl Config {
    /// CLI flags tienen prioridad → variables de entorno → defaults.
    pub fn from_cli_and_env(cli: &Cli) -> Self {
        let ai_base = dirs::home_dir()
            .expect("no home dir")
            .join(".local-ai");

        let default_web_dist = {
            // En release: buscar relativo al ejecutable
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent()?.parent().map(|p| p.join("Resources/web/dist")))
                .filter(|p| p.exists())
                // En dev: relativo al workspace
                .unwrap_or_else(|| {
                    std::env::current_dir()
                        .unwrap()
                        .parent()
                        .map(|p| p.join("web/dist"))
                        .unwrap_or_else(|| PathBuf::from("../web/dist"))
                })
        };

        Self {
            port: cli.port
                .or_else(|| std::env::var("PORT").ok().and_then(|v| v.parse().ok()))
                .unwrap_or(8080),

            llm_endpoint: cli.llm_endpoint.clone()
                .or_else(|| std::env::var("LLM_ENDPOINT").ok())
                .unwrap_or_else(|| "http://localhost:11434".into()),

            llm_model: std::env::var("LLM_MODEL").unwrap_or_default(),

            web_dist: cli.web_dist.clone()
                .or_else(|| std::env::var("WEB_DIST").ok().map(PathBuf::from))
                .unwrap_or(default_web_dist),

            plugins_dir: cli.plugins_dir.clone()
                .or_else(|| std::env::var("PLUGINS_DIR").ok().map(PathBuf::from))
                .unwrap_or_else(|| ai_base.join("plugins")),

            ai_base,
        }
    }

    pub fn uploads_dir(&self)   -> PathBuf { self.ai_base.join("uploads") }
    pub fn workspace_dir(&self) -> PathBuf { self.ai_base.join("workspace") }
    pub fn projects_dir(&self)  -> PathBuf { self.ai_base.join("projects") }
    pub fn knowledge_dir(&self) -> PathBuf { self.ai_base.join("knowledge") }
    pub fn settings_path(&self) -> PathBuf { self.ai_base.join("settings.json") }

    /// Carpeta por-proyecto: `~/.local-ai/projects/{doc_hash}/`.
    /// Contiene `original.{ext}`, `meta.json` y `outputs/`.
    pub fn project_dir(&self, doc_hash: &str) -> PathBuf {
        self.projects_dir().join(doc_hash)
    }
    pub fn project_outputs_dir(&self, doc_hash: &str) -> PathBuf {
        self.project_dir(doc_hash).join("outputs")
    }
}
