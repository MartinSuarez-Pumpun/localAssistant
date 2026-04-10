use std::sync::Arc;
use std::time::Duration;
use serde_json::Value;
use tokio::process::Command;
use tokio::time::timeout;

use crate::config::Config;

pub async fn run_command(args: &Value, config: &Arc<Config>) -> anyhow::Result<String> {
    let cmd = args["command"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("falta 'command'"))?;

    let workdir = args["workdir"]
        .as_str()
        .map(|d| config.workspace_dir().join(d))
        .unwrap_or_else(|| config.workspace_dir());

    let output = timeout(
        Duration::from_secs(30),
        Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .current_dir(&workdir)
            .output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("timeout (30s)"))?
    .map_err(|e| anyhow::anyhow!("error ejecutando comando: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let mut result = String::new();
    if !stdout.is_empty() { result.push_str(&stdout); }
    if !stderr.is_empty() {
        if !result.is_empty() { result.push('\n'); }
        result.push_str("[stderr]\n");
        result.push_str(&stderr);
    }
    if result.is_empty() { result.push_str("(sin salida)"); }

    Ok(result)
}