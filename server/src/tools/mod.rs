pub mod files;
pub mod readability;
mod shell;

use std::sync::Arc;
use crate::config::Config;

/// Despacha una tool call por nombre.
/// Equivale a `_exec_tool()` de serve.py.
pub async fn exec(
    name: &str,
    arguments: &str,
    config: Arc<Config>,
) -> anyhow::Result<String> {
    let args: serde_json::Value = serde_json::from_str(arguments)
        .unwrap_or(serde_json::Value::Object(Default::default()));

    match name {
        "read_file"    => files::read_file(&args, &config).await,
        "write_file"   => files::write_file(&args, &config).await,
        "edit_file"    => files::edit_file(&args, &config).await,
        "list_files"   => files::list_files(&args, &config).await,
        "search_files" => files::search_files(&args, &config).await,
        "delete_file"  => files::delete_file(&args, &config).await,
        "make_dir"     => files::make_dir(&args, &config).await,
        "run_command"  => shell::run_command(&args, &config).await,
        "create_docx"  => files::create_docx(&args, &config).await,
        "read_docx"    => files::read_docx(&args, &config).await,
        "create_pdf"   => files::create_pdf(&args, &config).await,
        other => anyhow::bail!("tool desconocida: {other}"),
    }
}