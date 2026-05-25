use crate::config::models::AppConfig;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// Checks if python3 is available on the system PATH.
/// Returns false if not found — when false, execute_python is not registered
/// as a tool so the LLM never sees it.
pub fn is_python_available() -> bool {
    std::process::Command::new("python3")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Executes arbitrary Python code supplied by the LLM.
///
/// The code is written to a temporary file and executed via `python3`.
/// Security is provided by:
///   1. Docker container isolation (the primary sandbox)
///   2. Dual LLM semantic verification (Phase 3)
///   3. CASS risk classification (Critical → always requires Dual LLM)
///
/// No AST-level sandboxing, no restricted builtins, no blocked modules —
/// those approaches proved brittle and incomplete in practice.
pub async fn execute_python(
    args: HashMap<String, Value>,
    config: Arc<AppConfig>,
) -> anyhow::Result<Value> {
    let code = match args.get("code") {
        Some(Value::String(s)) => s.clone(),
        Some(other) => {
            return Err(anyhow::anyhow!(
                "Invalid type for 'code': expected a string, got {}. \
                 Provide the Python source code as a single string.",
                other,
            ));
        }
        None => {
            return Err(anyhow::anyhow!(
                "Missing 'code' argument. Provide the Python source code to execute."
            ));
        }
    };

    if code.trim().is_empty() {
        return Err(anyhow::anyhow!("'code' must not be empty."));
    }

    // Write code to a temporary file so we can pass it to `python3` directly.
    // Using a file avoids shell injection and command-line length limits.
    let tmp_file = tempfile::Builder::new()
        .prefix("llsc_py_")
        .suffix(".py")
        .tempfile()
        .map_err(|e| anyhow::anyhow!("Failed to create temporary file: {}", e))?;

    std::fs::write(tmp_file.path(), &code)
        .map_err(|e| anyhow::anyhow!("Failed to write code to temporary file: {}", e))?;

    let tmp_path = tmp_file.path().to_path_buf();

    // Execute: python3 -u <tempfile>
    // -u flag forces unbuffered stdout/stderr, ensuring real-time streaming.
    // PYTHONUNBUFFERED=1 env var is also set as a belt-and-suspenders approach.
    let timeout_secs = config.general.command_timeout;

    let mut child = match Command::new("python3")
        .arg("-u")
        .arg(&tmp_path)
        .env("PYTHONUNBUFFERED", "1")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // This shouldn't happen since we check availability at registration,
            // but handle it gracefully just in case.
            return Err(anyhow::anyhow!(
                "python3 not found in system PATH. \
                 Ensure python3 is installed and available."
            ));
        }
        Err(e) => return Err(anyhow::anyhow!("Failed to start python3: {}", e)),
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
                        stderr_res.push_str(&l);
                        stderr_res.push('\n');
                    }
                    Ok(None) => stderr_done = true,
                    Err(_) => stderr_done = true,
                }
            }
            _ = &mut sleep => {
                let _ = child.kill().await;
                // Clean up the temp file on timeout
                let _ = std::fs::remove_file(&tmp_path);
                return Err(anyhow::anyhow!(
                    "Python execution timed out after {} seconds",
                    timeout_secs
                ));
            }
        }
    }

    // Clean up temp file after execution
    let _ = std::fs::remove_file(&tmp_path);

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
