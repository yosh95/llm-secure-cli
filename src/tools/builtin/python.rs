use crate::config::models::AppConfig;
use crate::core::session::SessionCancel;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// Checks if python3 is available on the system PATH.
/// Returns false if not found — when false, `execute_python` is not registered
/// as a tool so the LLM never sees it.
#[must_use]
pub fn is_python_available() -> bool {
    std::process::Command::new("python3")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// RAII guard that ensures the Python subprocess is killed and the temp file
/// is cleaned up when dropped (including on Ctrl+C / Future cancellation).
///
/// Without this guard, `tokio::process::Child` is silently detached on drop,
/// leaving orphan processes that accumulate and can intercept subsequent
/// Ctrl+C signals, making the shell unresponsive to interrupts.
struct PythonProcessGuard {
    child: Option<tokio::process::Child>,
    tmp_path: std::path::PathBuf,
}

impl PythonProcessGuard {
    fn new(child: tokio::process::Child, tmp_path: std::path::PathBuf) -> Self {
        Self {
            child: Some(child),
            tmp_path,
        }
    }

    /// Take ownership of the child for normal completion (Drop won't kill it).
    fn take_child(&mut self) -> Option<tokio::process::Child> {
        self.child.take()
    }
}

impl Drop for PythonProcessGuard {
    fn drop(&mut self) {
        if let Some(ref mut child) = self.child {
            // Send SIGKILL to prevent orphan processes.
            let _ = child.start_kill();
            // Try to reap the child synchronously (non-blocking).
            // Since we just killed it with start_kill(), try_wait() should
            // immediately return the exit status.
            let _ = child.try_wait();
        }
        // Best-effort cleanup of the temp file.
        let _ = std::fs::remove_file(&self.tmp_path);
    }
}

/// Executes arbitrary Python code supplied by the LLM.
///
/// The code is written to a temporary file and executed via `python3`.
/// Security is provided by:
///   1. Docker container isolation (the primary sandbox)
///   2. Verifier Committee semantic verification (Phase 3)
///   3. CASS risk classification (Critical -> always requires Verifier)
///
/// No AST-level sandboxing, no restricted builtins, no blocked modules --
/// those approaches proved brittle and incomplete in practice.
pub async fn execute_python(
    args: HashMap<String, Value>,
    config: Arc<AppConfig>,
) -> anyhow::Result<Value> {
    let code = match args.get("code") {
        Some(Value::String(s)) => s.clone(),
        Some(other) => {
            return Err(anyhow::anyhow!(
                "Invalid type for 'code': expected a string, got {other}.                  Provide the Python source code as a single string.",
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
        .map_err(|e| anyhow::anyhow!("Failed to create temporary file: {e}"))?;

    std::fs::write(tmp_file.path(), &code)
        .map_err(|e| anyhow::anyhow!("Failed to write code to temporary file: {e}"))?;

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
            // Clean up temp file before returning error
            let _ = std::fs::remove_file(&tmp_path);
            return Err(anyhow::anyhow!(
                "python3 not found in system PATH.                  Ensure python3 is installed and available."
            ));
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(anyhow::anyhow!("Failed to start python3: {e}"));
        }
    };

    // Set up stdout/stderr readers BEFORE moving child into the guard.
    // The readers take ownership of the pipes, so moving `child` afterwards is safe.
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

    // Now move the child into the RAII guard.
    // If this function exits early (timeout, Ctrl+C cancellation via outer select!),
    // the guard's Drop will kill the subprocess and clean up the temp file.
    let mut guard = PythonProcessGuard::new(child, tmp_path);

    let mut stdout_res = String::new();
    let mut stderr_res = String::new();

    let timeout_duration = Duration::from_secs(timeout_secs);
    let sleep = tokio::time::sleep(timeout_duration);
    tokio::pin!(sleep);

    let mut stdout_done = false;
    let mut stderr_done = false;

    // Subscribe to Ctrl+C via the global SIGINT watcher (no race with outer select!
    // because we use a synchronous value check, not an async branch).
    let cancel_token = SessionCancel::new();
    let mut cancel_rx = cancel_token.receiver();
    let cancel_base = *cancel_rx.borrow();

    while !stdout_done || !stderr_done {
        // Synchronous Ctrl+C check — no race with the outer select! in phase3_execution.rs.
        // If a signal arrived since we last checked, return partial output immediately.
        if *cancel_rx.borrow() != cancel_base {
            // Guard's Drop will kill the child and clean up the temp file.
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
                "note": "Python execution was interrupted by user (Ctrl+C) — the output may be incomplete (process was killed)."
            }));
        }

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
            _ = cancel_rx.changed() => {
                // Ctrl+C while waiting on I/O — same handler as sync check above.
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
                    "note": "Python execution was interrupted by user (Ctrl+C) — the output above may be incomplete (process was killed)."
                }));
            }
            () = &mut sleep => {
                // Guard's Drop will kill the child and clean up the temp file.
                // Return partial output with a note about the timeout.
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
                        "Python execution timed out after {} seconds — the output above may be incomplete (process was killed).",
                        timeout_secs
                    )
                }));
            }
        }
    }

    // Take the child back from the guard so Drop won't kill it
    // (it's about to exit naturally -- we just need to reap its status).
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
    // guard drops here: child is None (taken), so only temp file cleanup runs.
}
