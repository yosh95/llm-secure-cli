use crate::config::models::AppConfig;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// RAII guard that ensures the temp file is cleaned up when dropped,
/// **and** that the Python process is killed + reaped so it doesn't leak.
///
/// # Process lifecycle
///
/// 1. Python is spawned in a **new process group** (Unix only, via
///    `process_group(0)`).  This ensures Ctrl+C (SIGINT from the terminal)
///    hits only llsc, not Python directly — we want llsc to stay in control.
///
/// 2. On **normal exit**: `try_reap()` (called from the poll loop) reaps the
///    zombie.  The subsequent `Drop` calls `kill()` (no-op for exited process)
///    and `wait()` (returns `Err` for already-reaped child, ignored by `let _`).
///
/// 3. On **timeout / Ctrl+C / Drop**: `child.kill()` sends SIGKILL (Unix) or
///    TerminateProcess (Windows), then `child.wait()` reaps.  Python is gone.
///
/// 4. **Grandchild processes** (e.g. `subprocess.run()` inside the Python
///    code) become orphans — that is acceptable.  If the LLM wants a process
///    to survive beyond the tool-call, it must use `start_new_session=True`
///    (which places the grandchild in a separate session / process group,
///    immune to our `kill()`).
struct PythonProcessGuard {
    child: Option<std::process::Child>,
    tmp_path: std::path::PathBuf,
}

impl PythonProcessGuard {
    fn new(child: std::process::Child, tmp_path: std::path::PathBuf) -> Self {
        Self {
            child: Some(child),
            tmp_path,
        }
    }

    /// Try to reap the child if it has already exited (non-blocking).
    fn try_reap(&mut self) -> Option<std::process::ExitStatus> {
        self.child.as_mut()?.try_wait().ok().flatten()
    }
}

impl Drop for PythonProcessGuard {
    fn drop(&mut self) {
        if let Some(ref mut child) = self.child {
            let _ = child.kill();
            // Poll with try_wait() for up to 5 seconds.  This avoids
            // blocking the main thread indefinitely if the child is
            // stuck (e.g. a grandchild holding a pipe open).
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(_status) = child.try_wait().unwrap_or(None) {
                    break; // child reaped
                }
                if Instant::now() >= deadline {
                    break; // give up — init process will reap eventually
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        let _ = std::fs::remove_file(&self.tmp_path);
    }
}

/// Read from a pipe line-by-line, forwarding to a terminal handle while
/// also accumulating into a shared buffer.  The terminal lock is acquired
/// per-line (not held across the entire read), so the main thread can also
/// print to the terminal without deadlocking.
fn read_pipe_to_terminal_and_buffer<R: std::io::Read + Send + 'static>(
    reader: R,
    buffer: Arc<Mutex<String>>,
    use_stdout: bool,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buf_reader = BufReader::new(reader);
        let mut line = String::new();
        if use_stdout {
            let term = std::io::stdout();
            loop {
                line.clear();
                match buf_reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let mut handle = term.lock();
                        let _ = handle.write_all(line.as_bytes());
                        let _ = handle.flush();
                        // Lock released here (handle dropped)
                        if let Ok(mut g) = buffer.lock() {
                            g.push_str(&line);
                        }
                    }
                }
            }
        } else {
            let term = std::io::stderr();
            loop {
                line.clear();
                match buf_reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let mut handle = term.lock();
                        let _ = handle.write_all(line.as_bytes());
                        let _ = handle.flush();
                        // Lock released here (handle dropped)
                        if let Ok(mut g) = buffer.lock() {
                            g.push_str(&line);
                        }
                    }
                }
            }
        }
    })
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

    // Save and restore terminal state around the entire subprocess lifecycle.
    let _term_guard = crate::utils::TerminalGuard::new();

    // Spawn Python in a new process group so that Ctrl+C (SIGINT from the
    // terminal) hits llsc only, not Python directly.  llsc remains in control
    // and can clean up via `kill()` + `wait()`.  On Windows `process_group(0)`
    // is not available (it's a Unix-only extension method) and is simply not
    // called — the `#[cfg]` gates ensure that.
    #[cfg(unix)]
    let mut child = {
        use std::os::unix::process::CommandExt;
        match Command::new(&python_bin)
            .arg("-u")
            .arg(&tmp_path)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .process_group(0)
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
        }
    };

    #[cfg(not(unix))]
    let mut child = {
        match Command::new(&python_bin)
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

    // Reader threads: acquire terminal lock per-line, not for the entire
    // lifetime of the thread.  This prevents deadlock when the main thread
    // calls println!() while a reader thread holds stdout().lock().
    let h_out = read_pipe_to_terminal_and_buffer(stdout, stdout_buf.clone(), true);
    let h_err = read_pipe_to_terminal_and_buffer(stderr, stderr_buf.clone(), false);

    let mut guard = PythonProcessGuard::new(child, tmp_path);

    let max_lines = config.general.max_output_lines;
    let max_chars = config.general.max_output_chars;
    let truncate = |s: &str| crate::tools::executor_utils::truncate_output(s, max_lines, max_chars);

    let timeout = Duration::from_secs(timeout_secs);
    let start = Instant::now();
    let cancel_gen = crate::core::session::cancel_generation();

    // Poll the child for completion while staying responsive to Ctrl+C and
    // enforcing the timeout.  On early return, `guard` is dropped which
    // triggers `kill()` + `wait()` on the child.
    let status = loop {
        // Try to reap the child if it has already exited.
        if let Some(status) = guard.try_reap() {
            break status;
        }

        if crate::core::session::cancelled_since(cancel_gen) {
            // Drop guard here so kill+wait happens before we return.
            drop(guard);
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
            // Drop guard here so kill+wait happens before we return.
            drop(guard);
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
