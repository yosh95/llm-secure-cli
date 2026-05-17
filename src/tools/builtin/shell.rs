use crate::config::models::AppConfig;
use colored::Colorize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// Executes a system command directly without a shell.
/// 'argv' is an array where argv[0] is the command name and
/// the remaining elements are its arguments.
/// Architecturally, we rely on Phase 3 (Dual LLM) for intent verification.
pub async fn execute_command(
    args: HashMap<String, Value>,
    config: Arc<AppConfig>,
) -> anyhow::Result<Value> {
    // Extract argv: first element is the program name, rest are arguments
    let argv: Vec<String> = match args.get("argv") {
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        Some(other) => {
            return Err(anyhow::anyhow!(
                "Invalid type for 'argv': expected an array of strings, got {}. \
                 'argv' must be an array where the first element is the command name \
                 and subsequent elements are its arguments. \
                 Example: [\"git\", \"status\"] or [\"echo\", \"hello\", \"world\"].",
                other,
            ));
        }
        None => {
            return Err(anyhow::anyhow!(
                "Missing 'argv' argument. 'argv' must be an array where \
                 the first element is the command name (e.g. [\"git\", \"status\"])."
            ));
        }
    };

    if argv.is_empty() {
        return Err(anyhow::anyhow!(
            "'argv' must not be empty. Provide at least the command name as the first element."
        ));
    }

    let program = &argv[0];
    let cmd_args: &[String] = &argv[1..];

    // 1. Static Analysis (Minimalist/Semantic focus)
    // We strictly use Command::new which avoids shell-injection by design.
    // Semantic verification has already been handled by Phase 3 (Dual LLM).

    // 2. Execution with Timeout
    let timeout_secs = config.general.command_timeout;

    let mut cmd = Command::new(program);
    cmd.args(cmd_args);

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

    let mut stdout_reader = BufReader::new(
        child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to open stdout"))?,
    )
    .lines();
    let mut stderr_reader = BufReader::new(
        child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to open stderr"))?,
    )
    .lines();

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
            "stdout": crate::tools::executor_utils::truncate_output(
                &stdout_res,
                config.general.max_output_lines,
                config.general.max_output_chars
            ),
            "stderr": crate::tools::executor_utils::truncate_output(
                &stderr_res,
                config.general.max_output_lines,
                config.general.max_output_chars
            ),
            "exit_code": status.code().unwrap_or(-1)
        })),
        Err(e) => Err(anyhow::anyhow!("Execution error: {}", e)),
    }
}
