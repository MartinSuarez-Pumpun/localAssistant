use std::path::{Path, PathBuf};
use std::sync::Arc;
use serde_json::Value;
use walkdir::WalkDir;

use crate::config::Config;

// ─── Resolución de rutas (fuzzy, igual que serve.py) ─────────────────────────

/// Intenta resolver una ruta dada por el LLM.
/// Primero prueba exacta; luego fuzzy Levenshtein contra el árbol de dirs.
pub fn resolve_path(raw: &str, config: &Config) -> PathBuf {
    // Ruta absoluta — usarla directamente
    let p = Path::new(raw);
    if p.is_absolute() && p.exists() {
        return p.to_path_buf();
    }

    // Relativa al workspace o uploads
    for base in [config.workspace_dir(), config.uploads_dir(), config.knowledge_dir()] {
        let candidate = base.join(raw);
        if candidate.exists() {
            return candidate;
        }
    }

    // Fuzzy: buscar el archivo cuyo nombre tenga menor distancia de edición
    let filename = p.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(raw);

    let mut best: Option<(usize, PathBuf)> = None;

    for base in [config.workspace_dir(), config.uploads_dir()] {
        for entry in WalkDir::new(&base).max_depth(4).into_iter().flatten() {
            if entry.file_type().is_file() {
                if let Some(name) = entry.file_name().to_str() {
                    let dist = strsim::levenshtein(filename, name);
                    let threshold = (filename.len() / 3).max(2);
                    if dist <= threshold {
                        if best.as_ref().map_or(true, |(d, _)| dist < *d) {
                            best = Some((dist, entry.path().to_path_buf()));
                        }
                    }
                }
            }
        }
    }

    best.map(|(_, p)| p).unwrap_or_else(|| PathBuf::from(raw))
}

// ─── Tools ────────────────────────────────────────────────────────────────────

pub async fn read_file(args: &Value, config: &Arc<Config>) -> anyhow::Result<String> {
    let path_raw = args["path"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("falta 'path'"))?;

    let path = resolve_path(path_raw, config);

    // Detectar PDF y extraer texto
    let ext = path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if ext == "pdf" {
        return read_pdf_text(&path);
    }

    let content = tokio::fs::read_to_string(&path).await
        .map_err(|e| anyhow::anyhow!("no se pudo leer {}: {e}", path.display()))?;

    Ok(content)
}

fn read_pdf_text(path: &std::path::Path) -> anyhow::Result<String> {
    let bytes = std::fs::read(path)
        .map_err(|e| anyhow::anyhow!("no se pudo leer {}: {e}", path.display()))?;

    let text = pdf_extract::extract_text_from_mem(&bytes)
        .map_err(|e| anyhow::anyhow!("error extrayendo texto del PDF: {e}"))?;

    if text.trim().is_empty() {
        anyhow::bail!("el PDF no tiene texto extraíble (puede ser una imagen escaneada)");
    }

    Ok(text)
}

pub async fn write_file(args: &Value, config: &Arc<Config>) -> anyhow::Result<String> {
    let path_raw = args["path"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("falta 'path'"))?;
    let content = args["content"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("falta 'content'"))?;

    let path = resolve_path(path_raw, config);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&path, content).await
        .map_err(|e| anyhow::anyhow!("no se pudo escribir {}: {e}", path.display()))?;

    Ok(format!("escrito: {}", path.display()))
}

pub async fn edit_file(args: &Value, config: &Arc<Config>) -> anyhow::Result<String> {
    let path_raw = args["path"].as_str().ok_or_else(|| anyhow::anyhow!("falta 'path'"))?;
    let old = args["old_str"].as_str().ok_or_else(|| anyhow::anyhow!("falta 'old_str'"))?;
    let new = args["new_str"].as_str().ok_or_else(|| anyhow::anyhow!("falta 'new_str'"))?;

    let path = resolve_path(path_raw, config);
    let content = tokio::fs::read_to_string(&path).await
        .map_err(|e| anyhow::anyhow!("no se pudo leer {}: {e}", path.display()))?;

    if !content.contains(old) {
        anyhow::bail!("cadena no encontrada en {}", path.display());
    }

    let updated = content.replacen(old, new, 1);
    tokio::fs::write(&path, &updated).await?;
    Ok(format!("editado: {}", path.display()))
}

pub async fn list_files(args: &Value, config: &Arc<Config>) -> anyhow::Result<String> {
    let dir_raw = args["path"].as_str().unwrap_or(".");
    let path = resolve_path(dir_raw, config);

    let mut entries = vec![];
    if let Ok(mut rd) = tokio::fs::read_dir(&path).await {
        while let Ok(Some(entry)) = rd.next_entry().await {
            if let Some(name) = entry.file_name().to_str() {
                entries.push(name.to_string());
            }
        }
    }
    entries.sort();
    Ok(entries.join("\n"))
}

pub async fn search_files(args: &Value, config: &Arc<Config>) -> anyhow::Result<String> {
    let query = args["query"].as_str().ok_or_else(|| anyhow::anyhow!("falta 'query'"))?;
    let dir_raw = args["path"].as_str().unwrap_or(".");
    let base = resolve_path(dir_raw, config);

    let query_lower = query.to_lowercase();
    let mut matches = vec![];

    for entry in WalkDir::new(&base).max_depth(5).into_iter().flatten() {
        if entry.file_type().is_file() {
            let path = entry.path();
            // Búsqueda en nombre
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.to_lowercase().contains(&query_lower) {
                    matches.push(path.to_string_lossy().to_string());
                    continue;
                }
            }
            // Búsqueda en contenido (solo archivos de texto pequeños)
            if let Ok(meta) = entry.metadata() {
                if meta.len() < 512 * 1024 {
                    if let Ok(text) = std::fs::read_to_string(path) {
                        if text.to_lowercase().contains(&query_lower) {
                            matches.push(path.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }
    }

    if matches.is_empty() {
        Ok(format!("sin resultados para '{query}'"))
    } else {
        Ok(matches.join("\n"))
    }
}

pub async fn delete_file(args: &Value, config: &Arc<Config>) -> anyhow::Result<String> {
    let path_raw = args["path"].as_str().ok_or_else(|| anyhow::anyhow!("falta 'path'"))?;
    let path = resolve_path(path_raw, config);

    if path.is_dir() {
        tokio::fs::remove_dir_all(&path).await?;
    } else {
        tokio::fs::remove_file(&path).await?;
    }
    Ok(format!("eliminado: {}", path.display()))
}

pub async fn make_dir(args: &Value, config: &Arc<Config>) -> anyhow::Result<String> {
    let path_raw = args["path"].as_str().ok_or_else(|| anyhow::anyhow!("falta 'path'"))?;
    let path = config.workspace_dir().join(path_raw);
    tokio::fs::create_dir_all(&path).await?;
    Ok(format!("creado: {}", path.display()))
}

// ─── Documentos ───────────────────────────────────────────────────────────────

pub async fn create_docx(args: &Value, config: &Arc<Config>) -> anyhow::Result<String> {
    use docx_rs::*;

    let path_raw = args["path"].as_str().ok_or_else(|| anyhow::anyhow!("falta 'path'"))?;
    let content  = args["content"].as_str().ok_or_else(|| anyhow::anyhow!("falta 'content'"))?;

    let path = doc_path(path_raw, config, "docx");
    if let Some(p) = path.parent() { std::fs::create_dir_all(p)?; }

    let file = std::fs::File::create(&path)?;
    let mut doc = Docx::new();

    for line in content.lines() {
        let para = if let Some(h) = line.strip_prefix("# ") {
            Paragraph::new().add_run(Run::new().bold().size(40).add_text(h))
        } else if let Some(h) = line.strip_prefix("## ") {
            Paragraph::new().add_run(Run::new().bold().size(32).add_text(h))
        } else if let Some(h) = line.strip_prefix("### ") {
            Paragraph::new().add_run(Run::new().bold().size(28).add_text(h))
        } else if line.trim_start().starts_with("- ") || line.trim_start().starts_with("* ") {
            let text = line.trim_start().trim_start_matches(['-', '*']).trim();
            Paragraph::new().add_run(Run::new().add_text(format!("• {text}").as_str()))
        } else if line.trim().is_empty() {
            Paragraph::new()
        } else {
            // Limpiar markdown básico **bold** __bold__
            let clean = line.replace("**", "").replace("__", "").replace('*', "");
            Paragraph::new().add_run(Run::new().add_text(clean.as_str()))
        };
        doc = doc.add_paragraph(para);
    }

    doc.build().pack(file)?;
    Ok(format!("creado: {}", path.display()))
}

pub async fn read_docx(args: &Value, config: &Arc<Config>) -> anyhow::Result<String> {
    let path_raw = args["path"].as_str().ok_or_else(|| anyhow::anyhow!("falta 'path'"))?;
    let path = resolve_path(path_raw, config);

    let bytes  = std::fs::read(&path)
        .map_err(|e| anyhow::anyhow!("no se pudo leer {}: {e}", path.display()))?;
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| anyhow::anyhow!("no es un DOCX válido: {e}"))?;

    let mut xml = String::new();
    {
        let mut entry = archive.by_name("word/document.xml")
            .map_err(|_| anyhow::anyhow!("archivo DOCX sin document.xml"))?;
        std::io::Read::read_to_string(&mut entry, &mut xml)?;
    }

    let text = extract_xml_text(&xml);
    if text.trim().is_empty() {
        anyhow::bail!("el DOCX no contiene texto extraíble");
    }
    Ok(text)
}

pub async fn create_pdf(args: &Value, config: &Arc<Config>) -> anyhow::Result<String> {
    use printpdf::*;
    use std::io::BufWriter;

    let path_raw = args["path"].as_str().ok_or_else(|| anyhow::anyhow!("falta 'path'"))?;
    let content  = args["content"].as_str().ok_or_else(|| anyhow::anyhow!("falta 'content'"))?;

    let path = doc_path(path_raw, config, "pdf");
    if let Some(p) = path.parent() { std::fs::create_dir_all(p)?; }

    let title = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Document");
    let (doc, first_page, first_layer) =
        PdfDocument::new(title, Mm(210.0), Mm(297.0), "Layer 1");

    let font_reg  = doc.add_builtin_font(BuiltinFont::Helvetica)
        .map_err(|e| anyhow::anyhow!("fuente: {e}"))?;
    let font_bold = doc.add_builtin_font(BuiltinFont::HelveticaBold)
        .map_err(|e| anyhow::anyhow!("fuente bold: {e}"))?;

    // Agrupar líneas en páginas de ≤45 líneas
    let lines: Vec<&str> = content.lines().collect();
    let chunks: Vec<&[&str]> = lines.chunks(45).collect();

    for (i, chunk) in chunks.iter().enumerate() {
        let (page, layer) = if i == 0 {
            (first_page, first_layer)
        } else {
            let (p, l) = doc.add_page(Mm(210.0), Mm(297.0), "Layer 1");
            (p, l)
        };
        let layer = doc.get_page(page).get_layer(layer);
        let mut y = Mm(277.0_f32);

        for line in *chunk {
            let (text, bold, pt): (String, bool, f32) = if let Some(h) = line.strip_prefix("# ") {
                (h.replace("**", "").replace("__", ""), true, 18.0)
            } else if let Some(h) = line.strip_prefix("## ") {
                (h.replace("**", "").replace("__", ""), true, 14.0)
            } else if let Some(h) = line.strip_prefix("### ") {
                (h.replace("**", "").replace("__", ""), true, 12.0)
            } else {
                (line.replace("**", "").replace("__", "").replace('*', ""), false, 10.0)
            };

            if !text.trim().is_empty() {
                let f = if bold { &font_bold } else { &font_reg };
                layer.use_text(text, pt, Mm(20.0_f32), y, f);
            }
            y -= Mm(7.0_f32);
        }
    }

    let file = std::fs::File::create(&path)?;
    doc.save(&mut BufWriter::new(file))
        .map_err(|e| anyhow::anyhow!("error guardando PDF: {e}"))?;

    Ok(format!("creado: {}", path.display()))
}

// ─── Helpers privados ─────────────────────────────────────────────────────────

/// Devuelve la ruta destino para un documento: absoluta si ya lo es,
/// en workspace si es relativa. Si no tiene extensión, la añade.
fn doc_path(raw: &str, config: &Config, default_ext: &str) -> std::path::PathBuf {
    let p = std::path::Path::new(raw);
    let base = if p.is_absolute() {
        p.to_path_buf()
    } else {
        config.workspace_dir().join(raw)
    };
    if base.extension().is_none() {
        base.with_extension(default_ext)
    } else {
        base
    }
}

/// Extrae texto de document.xml de un DOCX escaneando etiquetas <w:t>.
fn extract_xml_text(xml: &str) -> String {
    // Añadir saltos de párrafo antes de escanear texto
    let expanded = xml.replace("</w:p>", "\n").replace("<w:br/>", "\n");
    let mut out  = String::new();
    let mut rest: &str = &expanded;

    while let Some(open) = rest.find("<w:t") {
        rest = &rest[open..];
        let Some(tag_end) = rest.find('>') else { break };
        rest = &rest[tag_end + 1..];
        let close = rest.find("</w:t>").unwrap_or(rest.len());
        out.push_str(&rest[..close]);
        rest = &rest[close..];
    }

    out.replace("&amp;", "&")
       .replace("&lt;", "<")
       .replace("&gt;", ">")
       .replace("&apos;", "'")
       .replace("&quot;", "\"")
}