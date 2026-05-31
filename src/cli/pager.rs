//! Pager module for handling long output.
//!
//! Supports three modes:
//! - `Disabled`: print everything directly.
//! - `Auto`: try `less -FRMe`, fall back to printing directly.
//! - `External(cmd)`: use a specific command, fall back to printing directly.
//!
//! In all cases, after the pager exits the full content is printed to stdout,
//! so the user sees the complete output as if no pager had been used — the
//! pager is purely a navigation convenience.
//!
//! # Configuration
//!
//! ```toml
//! [general]
//! pager = "auto"          # try less, fallback to direct print
//! pager = "less -FRMe"    # specific command
//! pager = ""              # disabled (default)
//! ```

use std::io::Write;
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Global pager configuration
// ---------------------------------------------------------------------------

static PAGER_CONFIG: OnceLock<PagerConfig> = OnceLock::new();

/// Set the global pager configuration.  Called once during initialisation.
pub fn set_pager_config(config: PagerConfig) {
    if PAGER_CONFIG.set(config).is_err() {
        tracing::warn!("PAGER_CONFIG already initialized — ignoring duplicate set");
    }
}

/// Get the current pager configuration.
pub fn get_pager_config() -> &'static PagerConfig {
    PAGER_CONFIG.get_or_init(|| PagerConfig::Disabled)
}

// ---------------------------------------------------------------------------
// Pager configuration
// ---------------------------------------------------------------------------

/// Pager operation mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PagerConfig {
    /// No paging — print everything directly (default).
    Disabled,
    /// Try `less`; if unavailable, print directly.
    Auto,
    /// Use a specific external command (e.g. `"less"`).
    /// Falls back to printing directly if the command cannot be launched.
    External(String),
}

impl PagerConfig {
    /// Parse from the `pager` config string.
    #[must_use]
    pub fn from_config_string(s: Option<&str>) -> Self {
        match s {
            None | Some("") => PagerConfig::Disabled,
            Some("auto") => PagerConfig::Auto,
            Some(cmd) => PagerConfig::External(cmd.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Page content if it exceeds a reasonable threshold.
///
/// * `term_height` — current terminal height in rows.  If 0 (pipe / non-TTY),
///   content is printed directly.
pub fn page_output(content: &str, term_height: u16) {
    if term_height == 0 {
        // Not a terminal — print directly.
        print!("{content}");
        return;
    }

    let line_count = content.lines().count();
    // Use 70% of terminal height as the threshold to avoid a full-screen
    // flicker for content that *almost* fits.
    let threshold = ((term_height as usize) * 70 / 100).max(1);

    if line_count <= threshold {
        print!("{content}");
        return;
    }

    let pager = get_pager_config();

    // Run the pager (which uses the alternate screen).  After it exits the
    // alternate screen is destroyed, so we re-print the full content so the
    // user sees everything as if no pager had been used.
    match pager {
        PagerConfig::Disabled => {
            print!("{content}");
        }
        PagerConfig::Auto => {
            try_less_pager(content);
            print!("{content}");
        }
        PagerConfig::External(cmd) => {
            try_external_pager(cmd, content);
            print!("{content}");
        }
    }
}

// ---------------------------------------------------------------------------
// External pager
// ---------------------------------------------------------------------------

/// Spawn `less` with a friendly status-line prompt so users unfamiliar
/// with `less` know how to navigate and exit.
#[allow(clippy::suspicious_command_arg_space)]
fn try_less_pager(content: &str) -> bool {
    use std::process::{Command, Stdio};

    // -F  quit if content fits on one screen
    // -R  interpret ANSI colour escapes
    // -Pm set the status-line prompt (default is just ":", which confuses newcomers)
    let mut child = match Command::new("less")
        .arg("-FRMe")
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return false,
    };

    if let Some(mut stdin) = child.stdin.take() {
        if stdin.write_all(content.as_bytes()).is_err() {
            if let Err(e) = child.wait() {
                tracing::warn!("Failed to wait for pager child process: {}", e);
            }
            return false;
        }
        drop(stdin);
    }

    child.wait().is_ok()
}

/// Try to spawn an external pager, piping `content` to its stdin.
/// Returns `true` on success (but the caller can safely ignore this — we
/// always print the content afterwards).
fn try_external_pager(cmd: &str, content: &str) -> bool {
    use std::process::{Command, Stdio};

    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.is_empty() {
        return false;
    }

    let mut child = match Command::new(parts[0])
        .args(&parts[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return false,
    };

    if let Some(mut stdin) = child.stdin.take() {
        if stdin.write_all(content.as_bytes()).is_err() {
            // Pipe write failed — the pager may have exited early.
            if let Err(e) = child.wait() {
                tracing::warn!("Failed to wait for pager child process: {}", e);
            }
            return false;
        }
        // Drop stdin to close the pipe and let the pager know we're done.
        drop(stdin);
    }

    child.wait().is_ok()
}
