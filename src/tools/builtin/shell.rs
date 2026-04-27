use crate::config::CONFIG_MANAGER;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::Duration;
use tokio::process::Command;

/// Executes a system command directly without a shell.
/// Architecturally, we rely on Phase 2 (Dual LLM) for intent verification.
pub async fn execute_command(args: HashMap<String, Value>) -> anyhow::Result<Value> {
    let program = args
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'command' argument"))?;

    let cmd_args: Vec<String> = match args.get("args") {
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        _ => Vec::new(),
    };

    // 1. Static Analysis (Minimalist/Semantic focus)
    let (safe, violations) =
        crate::security::static_analyzer::StaticAnalyzer::check(program, &cmd_args);
    if !safe {
        return Err(anyhow::anyhow!(
            "Security Blocked: {}",
            violations.join(", ")
        ));
    }

    // 2. Execution with Timeout
    let config = CONFIG_MANAGER.get_config();
    let timeout_secs = config.general.command_timeout;

    let mut cmd = Command::new(program);
    cmd.args(&cmd_args);

    // Structural Isolation: By using Command::new directly, we avoid shell-injection
    // vulnerabilities regardless of the operating system.
    let child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    match tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait_with_output()).await {
        Ok(Ok(output)) => Ok(json!({
            "stdout": String::from_utf8_lossy(&output.stdout),
            "stderr": String::from_utf8_lossy(&output.stderr),
            "exit_code": output.status.code().unwrap_or(-1)
        })),
        Ok(Err(e)) => Err(anyhow::anyhow!("Execution error: {}", e)),
        Err(_) => Err(anyhow::anyhow!(
            "Command timed out after {} seconds",
            timeout_secs
        )),
    }
}
