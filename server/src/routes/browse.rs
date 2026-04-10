use axum::{
    Router,
    extract::{Query, State},
    response::IntoResponse,
    routing::get,
};
use serde::{Deserialize, Serialize};

use crate::{config::Config, AppState};

#[derive(Deserialize)]
pub struct BrowseParams {
    path: Option<String>,
}

#[derive(Serialize)]
struct DirEntry {
    name: String,
    path: String,
    kind: String, // "root" | "dir" | "file"
    ext:  String,
    size: u64,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/browse", get(browse))
}

async fn browse(
    State(state): State<AppState>,
    Query(params): Query<BrowseParams>,
) -> impl IntoResponse {
    let path_str = params.path.unwrap_or_default();

    if path_str.is_empty() {
        return axum::Json(build_roots(&state.config)).into_response();
    }

    let path = std::path::Path::new(&path_str);

    // Seguridad: solo rutas dentro de los roots permitidos
    if !is_allowed(path, &state.config) {
        return axum::Json::<Vec<DirEntry>>(vec![]).into_response();
    }

    let mut entries: Vec<DirEntry> = vec![];

    if let Ok(mut rd) = tokio::fs::read_dir(path).await {
        while let Ok(Some(entry)) = rd.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') { continue; } // ocultos

            let Ok(meta) = entry.metadata().await else { continue };
            let is_dir   = meta.is_dir();
            let ext      = if is_dir { String::new() } else {
                entry.path().extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase()
            };

            entries.push(DirEntry {
                name,
                path: entry.path().to_string_lossy().to_string(),
                kind: if is_dir { "dir".into() } else { "file".into() },
                ext,
                size: if is_dir { 0 } else { meta.len() },
            });
        }
    }

    // Carpetas primero, luego por nombre
    entries.sort_by(|a, b| {
        b.kind.eq("dir").cmp(&a.kind.eq("dir")).then(a.name.cmp(&b.name))
    });

    axum::Json(entries).into_response()
}

// ─── Roots permitidos ─────────────────────────────────────────────────────────

fn build_roots(config: &Config) -> Vec<DirEntry> {
    let mut roots = vec![
        DirEntry {
            name: "LocalAI".into(),
            path: config.ai_base.to_string_lossy().into_owned(),
            kind: "root".into(),
            ext:  String::new(),
            size: 0,
        },
    ];

    // macOS: /Volumes/* (discos externos)
    #[cfg(target_os = "macos")]
    if let Ok(rd) = std::fs::read_dir("/Volumes") {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "Macintosh HD" { continue; }
            if entry.path().is_dir() {
                roots.push(DirEntry {
                    name,
                    path: entry.path().to_string_lossy().into_owned(),
                    kind: "root".into(),
                    ext:  String::new(),
                    size: 0,
                });
            }
        }
    }

    // Linux: /media/** y /mnt
    #[cfg(target_os = "linux")]
    for base in ["/media", "/mnt"] {
        if let Ok(rd) = std::fs::read_dir(base) {
            for entry in rd.flatten() {
                if entry.path().is_dir() {
                    // /media puede tener subdirectorios por usuario
                    if let Ok(rd2) = std::fs::read_dir(entry.path()) {
                        for e2 in rd2.flatten() {
                            if e2.path().is_dir() {
                                roots.push(DirEntry {
                                    name: e2.file_name().to_string_lossy().into_owned(),
                                    path: e2.path().to_string_lossy().into_owned(),
                                    kind: "root".into(),
                                    ext:  String::new(),
                                    size: 0,
                                });
                            }
                        }
                    } else {
                        roots.push(DirEntry {
                            name: entry.file_name().to_string_lossy().into_owned(),
                            path: entry.path().to_string_lossy().into_owned(),
                            kind: "root".into(),
                            ext:  String::new(),
                            size: 0,
                        });
                    }
                }
            }
        }
    }

    roots
}

fn is_allowed(path: &std::path::Path, config: &Config) -> bool {
    if path.starts_with(&config.ai_base) { return true; }

    #[cfg(target_os = "macos")]
    if path.starts_with("/Volumes") { return true; }

    #[cfg(target_os = "linux")]
    if path.starts_with("/media") || path.starts_with("/mnt") { return true; }

    false
}
