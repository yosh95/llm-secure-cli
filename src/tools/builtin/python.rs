use crate::config::models::AppConfig;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// RAII guard that ensures the temp file is cleaned up when dropped.
///
/// Unlike the previous implementation, this does NOT kill the child process.
/// The child is waited on (reaped) to prevent zombies, but is allowed to
/// continue running in the background.  This is important for use cases
/// such as CTF challenges where the Python code starts a long-lived
/// background server that must outlive the execute_python tool call.
///
/// When the entire Rust CLI process exits, the OS will clean up any
/// remaining child processes (typically via process-group signalling or
/// container teardown).
struct PythonProcessGuard {
    child: Option<ChildHandle>,
    tmp_path: std::path::PathBuf,
}

/// A wrapper around `std::process::Child` that does NOT kill the child
/// on drop.  It only reaps (waits for) the child to prevent zombie
/// processes, allowing background processes started by the Python code
/// to survive beyond the tool call.
struct ChildHandle {
    inner: Option<std::process::Child>,
}

impl ChildHandle {
    fn new(child: std::process::Child) -> Self {
        Self { inner: Some(child) }
    }

    /// Try to reap the child if it has already exited (non-blocking).
    fn try_reap(&mut self) -> Option<std::process::ExitStatus> {
        if let Some(ref mut c) = self.inner {
            match c.try_wait() {
                Ok(Some(status)) => {
                    self.inner = None;
                    Some(status)
                }
                Ok(None) => None,
                Err(_) => {
                    self.inner = None;
                    None
                }
            }
        } else {
            None
        }
    }

    /// Detach the child process: relinquish management entirely.
    /// The child becomes an orphan (adopted by init on Unix) and continues
    /// running independently.  No kill, no wait — we simply forget about it.
    fn detach(&mut self) {
        if let Some(child) = self.inner.take() {
            // `std::mem::forget` prevents the Child's Drop from running.
            // std::process::Child's Drop would kill the process, which we
            // explicitly do NOT want.  The OS will clean up the orphaned
            // process eventually (when it exits natively, or when this
            // parent process exits and the process group is cleaned up).
            std::mem::forget(child);
        }
    }
}

impl Drop for ChildHandle {
    fn drop(&mut self) {
        // If the child has already exited (inner set to None by try_reap),
        // there's nothing to do.
        // If the child is still running, we detach it — do NOT kill or
        // wait (which would block indefinitely).  The OS will clean up
        // the orphaned process when it exits natively.
        if let Some(child) = self.inner.take() {
            std::mem::forget(child);
        }
    }
}

impl PythonProcessGuard {
    fn new(child: std::process::Child, tmp_path: std::path::PathBuf) -> Self {
        Self {
            child: Some(ChildHandle::new(child)),
            tmp_path,
        }
    }
}

impl Drop for PythonProcessGuard {
    fn drop(&mut self) {
        // Do NOT kill the child process — just reap it if it has already
        // exited, or detach it so it can continue running in the background.
        if let Some(ref mut handle) = self.child {
            handle.try_reap();
        }
        let _ = std::fs::remove_file(&self.tmp_path);
    }
}

/// Take a snapshot of a shared output buffer, recovering from lock poisoning.
fn snapshot(buf: &Arc<Mutex<String>>) -> String {
    buf.lock()
        .map(|g| g.clone())
        .unwrap_or_else(|p| p.into_inner().clone())
}

/// Determines which Python interpreter to use.
/// Checks `python3` first, then falls back to `python`.
/// Returns `None` if neither is available.
fn find_python() -> Option<String> {
    // Try python3 first
    let python3_check = std::process::Command::new("python3")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    if python3_check.is_ok() {
        return Some("python3".to_string());
    }

    // Fall back to python
    let python_check = std::process::Command::new("python")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    if python_check.is_ok() {
        return Some("python".to_string());
    }

    None
}

/// Executes Python code supplied by the LLM.
///
/// The code is written to a temporary `.py` file and executed via:
///   - `python3` (preferred) on Unix
///   - `python` (fallback) if python3 is not available
///
/// Security is provided by:
///   1. Docker container isolation (the primary sandbox)
///   2. Verifier Committee semantic verification (Phase 2)
///   3. Human-in-the-loop approval (Phase 3)
///
/// This tool is only registered if a Python interpreter (`python3` or `python`)
/// is found in the system PATH.
pub fn execute_python(
    args: HashMap<String, Value>,
    config: Arc<AppConfig>,
) -> anyhow::Result<Value> {
    let code = match args.get("code") {
        Some(Value::String(s)) => s.clone(),
        Some(other) => {
            return Err(anyhow::anyhow!(
                "Invalid type for 'code': expected a string, got {other}.",
            ));
        }
        None => {
            return Err(anyhow::anyhow!(
                "Missing 'code' argument. Provide the Python code to execute."
            ));
        }
    };

    if code.trim().is_empty() {
        return Err(anyhow::anyhow!("'code' must not be empty."));
    }

    let python_bin = find_python().ok_or_else(|| {
        anyhow::anyhow!(
            "Neither 'python3' nor 'python' found in system PATH. Cannot execute Python code."
        )
    })?;

    let tmp_file = tempfile::Builder::new()
        .prefix("llsc_py_")
        .suffix(".py")
        .tempfile()
        .map_err(|e| anyhow::anyhow!("Failed to create temporary file: {e}"))?;

    // Write the Python code to the temp file
    std::fs::write(tmp_file.path(), &code)
        .map_err(|e| anyhow::anyhow!("Failed to write code to temporary file: {e}"))?;

    let tmp_path = tmp_file.path().to_path_buf();
    let timeout_secs = config.general.python_timeout;

    let mut child = match Command::new(&python_bin)
        .arg("-u")
        .arg(&tmp_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(anyhow::anyhow!("{} not found in system PATH.", python_bin));
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(anyhow::anyhow!("Failed to start {python_bin}: {e}"));
        }
    };

    let stdout = child.stdout.take().ok_or_else(|| {
        let _ = std::fs::remove_file(&tmp_path);
        anyhow::anyhow!("Failed to open stdout")
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        let _ = std::fs::remove_file(&tmp_path);
        anyhow::anyhow!("Failed to open stderr")
    })?;

    // Shared, incrementally-filled output buffers.  Reader threads append to
    // them so we can return whatever was captured so far even on timeout/Ctrl+C.
    let stdout_buf = Arc::new(Mutex::new(String::new()));
    let stderr_buf = Arc::new(Mutex::new(String::new()));

    let so = stdout_buf.clone();
    let h_out = std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let mut term_stdout = std::io::stdout().lock();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let _ = term_stdout.write_all(line.as_bytes());
                    let _ = term_stdout.flush();
                    // Also accumulate in the LLM buffer
                    if let Ok(mut g) = so.lock() {
                        g.push_str(&line);
                    }
                }
            }
        }
    });
    let se = stderr_buf.clone();
    let h_err = std::thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        let mut term_stderr = std::io::stderr().lock();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let _ = term_stderr.write_all(line.as_bytes());
                    let _ = term_stderr.flush();
                    // Also accumulate in the LLM buffer
                    if let Ok(mut g) = se.lock() {
                        g.push_str(&line);
                    }
                }
            }
        }
    });

    // Ensure terminal ISIG is enabled so Ctrl+C generates SIGINT.
    crate::utils::ensure_isig_enabled();

    let mut guard = PythonProcessGuard::new(child, tmp_path);

    let max_lines = config.general.max_output_lines;
    let max_chars = config.general.max_output_chars;
    let truncate = |s: &str| crate::tools::executor_utils::truncate_output(s, max_lines, max_chars);

    let timeout = Duration::from_secs(timeout_secs);
    let start = Instant::now();
    let cancel_gen = crate::core::session::cancel_generation();

    // Poll the child for completion while staying responsive to Ctrl+C and
    // enforcing the timeout.  The guard kills the child on early return.
    let status = loop {
        // Try to reap the child if it has already exited.
        if let Some(ref mut handle) = guard.child
            && let Some(status) = handle.try_reap()
        {
            break status;
        }

        if crate::core::session::cancelled_since(cancel_gen) {
            // Note: we do NOT kill the child process.  The Python code may
            // have started long-lived background processes (e.g. a CTF
            // challenge server) that should continue running even after
            // the tool call is interrupted.
            //
            // Explicitly detach so no kill/wait happens on drop.
            if let Some(ref mut handle) = guard.child {
                handle.detach();
            }
            guard.child = None;

            // Wait for reader threads to drain the pipes with a short timeout.
            let _ = h_out.join();
            let _ = h_err.join();

            return Ok(json!({
                "stdout": truncate(&snapshot(&stdout_buf)),
                "stderr": truncate(&snapshot(&stderr_buf)),
                "exit_code": serde_json::Value::Null,
                "note": "Execution was interrupted by user (Ctrl+C).",
                "_real_time_displayed": true
            }));
        }

        if start.elapsed() >= timeout {
            // Note: we do NOT kill the child process on timeout.  The Python
            // code may have started long-lived background processes (e.g. a
            // CTF challenge server) that should continue running even after
            // the tool call times out.
            //
            // Explicitly detach so no kill/wait happens on drop.
            if let Some(ref mut handle) = guard.child {
                handle.detach();
            }
            guard.child = None;

            // Wait for reader threads to drain the pipes.
            let _ = h_out.join();
            let _ = h_err.join();

            return Ok(json!({
                "stdout": truncate(&snapshot(&stdout_buf)),
                "stderr": truncate(&snapshot(&stderr_buf)),
                "exit_code": serde_json::Value::Null,
                "note": format!("Execution timed out after {} seconds.", timeout_secs),
                "_real_time_displayed": true
            }));
        }

        std::thread::sleep(Duration::from_millis(50));
    };

    // Child exited: wait for the reader threads to drain the pipes.
    let _ = h_out.join();
    let _ = h_err.join();

    Ok(json!({
        "stdout": truncate(&snapshot(&stdout_buf)),
        "stderr": truncate(&snapshot(&stderr_buf)),
        "exit_code": status.code().unwrap_or(-1),
        "_real_time_displayed": true
    }))
}
