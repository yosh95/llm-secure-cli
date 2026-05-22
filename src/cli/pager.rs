//! Pager module for handling long output.
//!
//! Supports three modes:
//! - `Disabled`: print everything directly (current behavior).
//! - `Auto`: try `less -FRSX`, fall back to built-in pager.
//! - `External(cmd)`: use a specific command, fall back to built-in pager.
//!
//! The built-in pager uses no alternate screen buffer — content scrolls
//! naturally from top to bottom, preserving terminal scrollback.
//!
//! # Configuration
//!
//! ```toml
//! [general]
//! pager = "auto"          # try less, fallback to builtin
//! pager = "less -FRSX"    # specific command
//! pager = ""              # disabled (default)
//! ```

use console::Term;
use std::io::{self, BufRead, Write};
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Global pager configuration
// ---------------------------------------------------------------------------

static PAGER_CONFIG: OnceLock<PagerConfig> = OnceLock::new();

/// Set the global pager configuration.  Called once during initialisation.
pub fn set_pager_config(config: PagerConfig) {
    let _ = PAGER_CONFIG.set(config);
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
    /// Try `less -FRSX`; if unavailable, use the built-in pager.
    Auto,
    /// Use a specific external command (e.g. `"less -FRSX"`).
    /// Falls back to built-in pager if the command cannot be launched.
    External(String),
}

impl PagerConfig {
    /// Parse from the `pager` config string.
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

    match pager {
        PagerConfig::Disabled => {
            print!("{content}");
        }
        PagerConfig::Auto => {
            if !try_external_pager("less -FRSX", content) {
                builtin_pager(content, term_height);
            }
        }
        PagerConfig::External(cmd) => {
            if !try_external_pager(cmd, content) {
                builtin_pager(content, term_height);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// External pager
// ---------------------------------------------------------------------------

/// Try to spawn an external pager, piping `content` to its stdin.
/// Returns `true` on success.
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
            let _ = child.wait();
            return false;
        }
        // Drop stdin to close the pipe and let the pager know we're done.
        drop(stdin);
    }

    child.wait().is_ok()
}

// ---------------------------------------------------------------------------
// Built-in pager
// ---------------------------------------------------------------------------

/// Built-in simple pager.
///
/// * No alternate screen buffer (content scrolls naturally).
/// * Space / Enter = next page.
/// * `q` = quit (remaining output is available via session log).
/// * `a` = print all remaining content at once.
fn builtin_pager(content: &str, term_height: u16) {
    let lines: Vec<&str> = content.lines().collect();
    // Reserve 2 lines: 1 for the prompt, 1 for visual breathing room.
    let page_size = (term_height as usize).saturating_sub(2).max(1);
    let total_pages = lines.len().div_ceil(page_size);

    if total_pages <= 1 {
        print!("{content}");
        return;
    }

    let mut start = 0usize;
    let mut show_all = false;

    while start < lines.len() {
        let end = if show_all {
            lines.len()
        } else {
            (start + page_size).min(lines.len())
        };

        // Print the current page.
        for &line in &lines[start..end] {
            println!("{line}");
        }

        if show_all {
            break;
        }

        start = end;
        if start >= lines.len() {
            break;
        }

        let current_page = start / page_size;
        // Print prompt to stderr so it doesn't interfere with potential
        // stdout redirection of the main content.
        {
            let prompt = format!(
                "[ {}/{} pages | Space/Enter=next, q=quit, a=all ] ",
                current_page, total_pages
            );
            use colored::Colorize;
            eprint!("{}", prompt.dimmed());
            let _ = io::stderr().flush();
        }

        match read_key_or_line() {
            PagerInput::Quit => break,
            PagerInput::ShowAll => show_all = true,
            PagerInput::Next | PagerInput::Unknown => { /* advance to next page */ }
            PagerInput::Eof => break,
        }

        // Erase the prompt line from stderr.
        eprint!("\r\x1b[K");
        let _ = io::stderr().flush();
    }
}

// ---------------------------------------------------------------------------
// Key input for built-in pager
// ---------------------------------------------------------------------------

enum PagerInput {
    Next,
    Quit,
    ShowAll,
    Unknown,
    Eof,
}

/// Read a single key from the terminal without requiring Enter.
///
/// Uses `console::Term::read_key()` when stdin is a TTY, falling back to
/// line-buffered `read_line` for non-TTY (pipe) scenarios.
fn read_key_or_line() -> PagerInput {
    use std::io::IsTerminal;

    if std::io::stdin().is_terminal() {
        let term = Term::stdout();
        match term.read_key() {
            Ok(key) => match key {
                console::Key::Char('q') | console::Key::Char('Q') => PagerInput::Quit,
                console::Key::Char('a') | console::Key::Char('A') => PagerInput::ShowAll,
                console::Key::Char(' ') | console::Key::Enter => PagerInput::Next,
                console::Key::Escape => PagerInput::Quit,
                console::Key::UnknownEscSeq(_) => PagerInput::Unknown,
                _ => PagerInput::Next,
            },
            Err(_) => PagerInput::Eof,
        }
    } else {
        // Fallback: read a full line (Space or Enter triggers next page).
        let mut buf = String::new();
        match io::stdin().lock().read_line(&mut buf) {
            Ok(0) => PagerInput::Eof,
            Ok(_) => {
                let trimmed = buf.trim().to_lowercase();
                if trimmed == "q" {
                    PagerInput::Quit
                } else if trimmed == "a" {
                    PagerInput::ShowAll
                } else {
                    PagerInput::Next
                }
            }
            Err(_) => PagerInput::Eof,
        }
    }
}
