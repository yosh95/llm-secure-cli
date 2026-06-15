use crate::config::models::AppConfig;
use crate::core::session::SessionCancel;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// RAII guard that ensures the subprocess is killed and the temp file
/// is cleaned up when dropped (including on Ctrl+C / Future cancellation).
struct ShellProcessGuard {
    child: Option<tokio::process::Child>,
    tmp_path: std::path::PathBuf,
}

impl ShellProcessGuard {
    fn new(child: tokio::process::Child, tmp_path: std::path::PathBuf) -> Self {
        Self {
            child: Some(child),
            tmp_path,
        }
    }

    fn take_child(&mut self) -> Option<tokio::process::Child> {
        self.child.take()
    }
}

impl Drop for ShellProcessGuard {
    fn drop(&mut self) {
        if let Some(ref mut child) = self.child {
            let _ = child.start_kill();
            let _ = child.try_wait();
        }
        let _ = std::fs::remove_file(&self.tmp_path);
    }
}

/// Executes shell commands supplied by the LLM.
///
/// The command is written to a temporary script file and executed via:
///   - `bash` (heredoc) on Unix — backtick-safe
///   - `cmd.exe` on Windows
///
/// Security is provided by:
///   1. Docker container isolation (the primary sandbox)
///   2. Verifier Committee semantic verification (Phase 2)
///   3. Human-in-the-loop approval (Phase 3)
pub async fn execute_shell(
    args: HashMap<String, Value>,
    config: Arc<AppConfig>,
) -> anyhow::Result<Value> {
    let command = match args.get("command") {
        Some(Value::String(s)) => s.clone(),
        Some(other) => {
            return Err(anyhow::anyhow!(
                "Invalid type for 'command': expected a string, got {other}.",
            ));
        }
        None => {
            return Err(anyhow::anyhow!(
                "Missing 'command' argument. Provide the shell command(s) to execute."
            ));
        }
    };

    if command.trim().is_empty() {
        return Err(anyhow::anyhow!("'command' must not be empty."));
    }

    let tmp_file = tempfile::Builder::new()
        .prefix("llsc_sh_")
        .suffix(if cfg!(windows) { ".bat" } else { ".sh" })
        .tempfile()
        .map_err(|e| anyhow::anyhow!("Failed to create temporary file: {e}"))?;

    // Write the script content
    let script = if cfg!(windows) {
        // Windows: cmd.exe /c <temp.bat>
        // @echo off suppresses command echo
        format!("@echo off\r\n{}\r\n", command)
    } else {
        // Unix: bash heredoc — backtick-safe
        // The 'SCRIPT_END' quoting prevents the outer shell from expanding
        // anything inside the heredoc, while the inner bash interprets
        // $(...), `...`, |, >, 2>&1, etc. normally.
        format!(
            "bash << 'SCRIPT_END'\nset -euo pipefail\n{}\nSCRIPT_END\n",
            command
        )
    };

    std::fs::write(tmp_file.path(), &script)
        .map_err(|e| anyhow::anyhow!("Failed to write script to temporary file: {e}"))?;

    let tmp_path = tmp_file.path().to_path_buf();
    let timeout_secs = config.general.command_timeout;

    // Platform-specific shell invocation
    let (shell, arg) = if cfg!(windows) {
        ("cmd.exe", "/c")
    } else {
        ("bash", tmp_path.to_str().unwrap_or(""))
    };

    let mut child = match Command::new(shell)
        .arg(arg)
        .arg(&tmp_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(anyhow::anyhow!(
                "{} not found in system PATH.",
                if cfg!(windows) { "cmd.exe" } else { "bash" }
            ));
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(anyhow::anyhow!("Failed to start {}: {e}", shell));
        }
    };

    let mut stdout_reader = BufReader::new(child.stdout.take().ok_or_else(|| {
        let _ = std::fs::remove_file(&tmp_path);
        anyhow::anyhow!("Failed to open stdout")
    })?)
    .lines();
    let mut stderr_reader = BufReader::new(child.stderr.take().ok_or_else(|| {
        let _ = std::fs::remove_file(&tmp_path);
        anyhow::anyhow!("Failed to open stderr")
    })?)
    .lines();

    let mut guard = ShellProcessGuard::new(child, tmp_path);

    let mut stdout_res = String::new();
    let mut stderr_res = String::new();

    let timeout_duration = Duration::from_secs(timeout_secs);
    let sleep = tokio::time::sleep(timeout_duration);
    tokio::pin!(sleep);

    let mut stdout_done = false;
    let mut stderr_done = false;

    let cancel_token = SessionCancel::new();
    let mut cancel_rx = cancel_token.receiver();
    let cancel_base = *cancel_rx.borrow();

    while !stdout_done || !stderr_done {
        if *cancel_rx.borrow() != cancel_base {
            return Ok(json!({
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
                "exit_code": serde_json::Value::Null,
                "note": "Execution was interrupted by user (Ctrl+C)."
            }));
        }

        tokio::select! {
            line = stdout_reader.next_line(), if !stdout_done => {
                match line {
                    Ok(Some(l)) => { stdout_res.push_str(&l); stdout_res.push('\n'); }
                    Ok(None) => stdout_done = true,
                    Err(_) => stdout_done = true,
                }
            }
            line = stderr_reader.next_line(), if !stderr_done => {
                match line {
                    Ok(Some(l)) => { stderr_res.push_str(&l); stderr_res.push('\n'); }
                    Ok(None) => stderr_done = true,
                    Err(_) => stderr_done = true,
                }
            }
            _ = cancel_rx.changed() => {
                return Ok(json!({
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
                    "exit_code": serde_json::Value::Null,
                    "note": "Execution was interrupted by user (Ctrl+C)."
                }));
            }
            () = &mut sleep => {
                return Ok(json!({
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
                    "exit_code": serde_json::Value::Null,
                    "note": format!(
                        "Execution timed out after {} seconds.",
                        timeout_secs
                    )
                }));
            }
        }
    }

    let mut child = guard
        .take_child()
        .ok_or_else(|| anyhow::anyhow!("Child process state inconsistent"))?;

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
        Err(e) => Err(anyhow::anyhow!("Execution error: {e}")),
    }
}
