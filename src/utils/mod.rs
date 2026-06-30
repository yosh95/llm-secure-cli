pub mod chat_logger;
pub mod http;
pub mod logging;
pub mod media;
pub mod session_store;

const HEX_CHARS: [char; 16] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f',
];

pub fn hex_encode(data: impl AsRef<[u8]>) -> String {
    let bytes = data.as_ref();
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX_CHARS[((b >> 4) & 0x0f) as usize]);
        s.push(HEX_CHARS[(b & 0x0f) as usize]);
    }
    s
}

pub fn format_number<T: std::fmt::Display>(n: T) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut result = String::new();

    for (i, &b) in bytes.iter().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(b as char);
    }
    result.chars().rev().collect()
}

/// Save the current terminal settings to a string (via `stty -g`).
/// Returns `None` if not on Unix or if `stty` is unavailable.
///
/// The saved string can be passed to [`restore_terminal_settings`] to
/// atomically restore the terminal to the saved state.
#[must_use]
pub fn save_terminal_settings() -> Option<String> {
    #[cfg(unix)]
    {
        let output = std::process::Command::new("stty")
            .args(["-g"])
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        if output.status.success() {
            let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    None
}

/// Restore terminal settings previously saved via [`save_terminal_settings`].
/// If `settings` is `None`, falls back to `stty sane` + `icanon` + `isig`.
pub fn restore_terminal_settings(_settings: Option<&str>) {
    #[cfg(unix)]
    {
        if let Some(s) = _settings
            && !s.is_empty()
        {
            let _ = std::process::Command::new("stty").arg(s).status();
        } else {
            restore_terminal();
        }
    }
}

/// Restore the terminal to cooked mode so that Ctrl+C (SIGINT) works correctly.
///
/// rustyline leaves the terminal in raw mode on some code paths (e.g. on
/// interrupt or error), which disables ISIG and makes Ctrl+C unable to
/// generate SIGINT.  This function resets the terminal to sane settings
/// using `stty sane` and explicitly re-enables `icanon` and `isig`.
pub fn restore_terminal() {
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("stty").args(["sane"]).status();
        let _ = std::process::Command::new("stty")
            .args(["icanon", "isig"])
            .status();
    }
}

/// Ensure the ISIG flag is enabled so that Ctrl+C generates SIGINT.
///
/// Call this before any blocking operation (tool execution, HTTP request)
/// that should be responsive to Ctrl+C.  This is a safety net in case
/// rustyline or another component left the terminal in raw mode.
///
/// Unlike [`restore_terminal`], this only sets `isig` without changing
/// other terminal flags — use it when you want to remain in raw-like mode
/// but still receive SIGINT on Ctrl+C.
pub fn ensure_isig_enabled() {
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("stty").args(["isig"]).status();
    }
}

/// RAII guard that saves the terminal state on creation and restores it
/// on drop.  Use this around any blocking operation that might temporarily
/// modify the terminal (subprocess execution, HTTP requests, etc.).
///
/// # Example
///
/// ```ignore
/// let _guard = crate::utils::TerminalGuard::new();
/// // ... run tool or HTTP request ...
/// drop(_guard); // terminal is restored here (also happens automatically)
/// ```
pub struct TerminalGuard {
    saved_settings: Option<String>,
}

impl TerminalGuard {
    /// Save the current terminal settings and switch to cooked mode
    /// with ISIG enabled (so Ctrl+C generates SIGINT).
    #[must_use]
    pub fn new() -> Self {
        let saved = save_terminal_settings();
        // Ensure we're in a state where Ctrl+C works.
        #[cfg(unix)]
        {
            let _ = std::process::Command::new("stty").args(["sane"]).status();
            let _ = std::process::Command::new("stty")
                .args(["icanon", "isig"])
                .status();
        }
        Self {
            saved_settings: saved,
        }
    }

    /// Explicitly restore the terminal now (same as dropping the guard).
    /// Calling this multiple times is safe — the second call is a no-op.
    pub fn restore(&mut self) {
        if let Some(ref s) = self.saved_settings.take() {
            restore_terminal_settings(Some(s));
        }
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

impl Default for TerminalGuard {
    fn default() -> Self {
        Self::new()
    }
}
