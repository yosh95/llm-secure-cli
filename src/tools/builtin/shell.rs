use crate::config::models::AppConfig;
use colored::Colorize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// Executes a system command directly without a shell.
/// Architecturally, we rely on Phase 2 (Dual LLM) for intent verification.
pub async fn execute_command(
    args: HashMap<String, Value>,
    config: Arc<AppConfig>,
) -> anyhow::Result<Value> {
    let program = args
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'command' argument"))?;

    let cmd_args: Vec<String> = match args.get("args") {
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        Some(other) => {
            return Err(anyhow::anyhow!(
                "Invalid type for 'args': expected an array of strings, got {}. \
                 If you have a single argument, wrap it in an array: \
                 e.g. {{\"args\": [\"{}\"]}} instead of {{\"args\": \"{}\"}}.",
                other,
                match other {
                    Value::String(s) => s.clone(),
                    v => v.to_string(),
                },
                match other {
                    Value::String(s) => s.clone(),
                    v => v.to_string(),
                },
            ));
        }
        None => Vec::new(),
    };

    // 1. Static Analysis (Minimalist/Semantic focus)
    // We strictly use Command::new which avoids shell-injection by design.
    // Semantic verification has already been handled by Phase 3 (Dual LLM).
    let cwd = args.get("cwd").and_then(|v| v.as_str());

    // 2. Execution with Timeout
    let timeout_secs = config.general.command_timeout;

    let mut cmd = Command::new(program);
    cmd.args(&cmd_args);
    if let Some(c) = cwd {
        cmd.current_dir(c);
    }

    // Structural Isolation: By using Command::new directly, we avoid shell-injection
    // vulnerabilities regardless of the operating system.
    let mut child = match cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(anyhow::anyhow!(
                "Command '{}' not found in system PATH. Please ensure the command is correct and installed. Do NOT include natural language or explanations in the command field.",
                program
            ));
        }
        Err(e) => return Err(anyhow::anyhow!("Failed to start process: {}", e)),
    };

    let mut stdout_reader =
        BufReader::new(child.stdout.take().expect("Failed to open stdout")).lines();
    let mut stderr_reader =
        BufReader::new(child.stderr.take().expect("Failed to open stderr")).lines();

    let mut stdout_res = String::new();
    let mut stderr_res = String::new();

    let timeout_duration = Duration::from_secs(timeout_secs);
    let sleep = tokio::time::sleep(timeout_duration);
    tokio::pin!(sleep);

    let mut stdout_done = false;
    let mut stderr_done = false;

    while !stdout_done || !stderr_done {
        tokio::select! {
            line = stdout_reader.next_line(), if !stdout_done => {
                match line {
                    Ok(Some(l)) => {
                        eprintln!("      {}", l.dimmed());
                        stdout_res.push_str(&l);
                        stdout_res.push('\n');
                    }
                    Ok(None) => stdout_done = true,
                    Err(_) => stdout_done = true,
                }
            }
            line = stderr_reader.next_line(), if !stderr_done => {
                match line {
                    Ok(Some(l)) => {
                        eprintln!("      {}", l.red());
                        stderr_res.push_str(&l);
                        stderr_res.push('\n');
                    }
                    Ok(None) => stderr_done = true,
                    Err(_) => stderr_done = true,
                }
            }
            _ = &mut sleep => {
                let _ = child.kill().await;
                return Err(anyhow::anyhow!(
                    "Command timed out after {} seconds",
                    timeout_secs
                ));
            }
        }
    }

    match child.wait().await {
        Ok(status) => Ok(json!({
            "stdout": crate::tools::executor_utils::truncate_output(&stdout_res),
            "stderr": crate::tools::executor_utils::truncate_output(&stderr_res),
            "exit_code": status.code().unwrap_or(-1)
        })),
        Err(e) => Err(anyhow::anyhow!("Execution error: {}", e)),
    }
}
