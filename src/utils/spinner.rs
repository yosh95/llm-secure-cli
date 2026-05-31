use std::io::Write;
use std::time::Duration;

/// A minimal spinner that displays an animated indicator while a task runs.
///
/// Uses only `tokio::time` and `std::io` — no external dependencies.
///
/// # Example
///
/// ```ignore
/// let spin = Spinner::start("Loading …");
/// do_work().await;
/// spin.finish("done");  // → "Loading ... 3.2s done"
/// ```
///
/// On early returns (e.g. `?` operator), `Drop` automatically cleans up the line.
pub struct Spinner {
    handle: Option<tokio::task::JoinHandle<()>>,
    msg: String,
    start: tokio::time::Instant,
}

// ── Terminal width ──
/// Get terminal width via `console::Term`. Falls back to 80.
fn terminal_width() -> u16 {
    console::Term::stdout()
        .size_checked()
        .map_or(80, |(_, cols)| cols)
}

/// Reserve space for: `spinner_char` (1-3 chars) + space + "XX.Xs" (6 chars)
const RESERVED_COLS: u16 = 12;

/// Truncate `msg` at the **beginning** so the tail (model name) is preserved.
/// e.g. "openai/gpt-4o-mini-longname..." → "…/gpt-4o-mini-longname"
fn truncate_msg(msg: &mut String, term_width: u16) {
    let max_msg_len = term_width.saturating_sub(RESERVED_COLS) as usize;
    if msg.len() > max_msg_len && max_msg_len > 3 {
        let suffix: String = msg.chars().skip(msg.len() - max_msg_len + 1).collect();
        *msg = "\u{2026}".to_string() + &suffix;
    }
}

// ── Spinner characters ──
/// Modern braille-pattern spinner (works everywhere, including Termux).
const SPINNER_CHARS: &[&str] = &[
    "\u{280B}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283C}", "\u{2834}", "\u{2826}", "\u{2827}",
    "\u{2807}", "\u{280F}",
];

// ── Cursor positioning ──
fn cursor_to_col1() -> &'static str {
    "\x1b[1G"
}

const ERASE_LINE: &str = "\x1b[2K";

// ── Implementation ──
impl Spinner {
    #[must_use]
    pub fn start(msg: &str) -> Self {
        let mut msg = msg.to_string();
        let term_w = terminal_width();
        truncate_msg(&mut msg, term_w);

        let msg_for_spawn = msg.clone();
        let goto = cursor_to_col1();

        print!(
            "{erase}{goto}{sp} {msg} 0.0s",
            erase = ERASE_LINE,
            goto = goto,
            sp = SPINNER_CHARS[0],
            msg = msg
        );
        std::io::stdout().flush().ok();

        let handle = tokio::spawn(async move {
            let start = tokio::time::Instant::now();
            let mut idx: usize = 1;
            let goto = cursor_to_col1();
            loop {
                tokio::time::sleep(Duration::from_millis(80)).await;
                let elapsed = start.elapsed();
                print!(
                    "{erase}{goto}{sp} {msg} {elapsed:.1}s",
                    erase = ERASE_LINE,
                    goto = goto,
                    sp = SPINNER_CHARS[idx],
                    msg = msg_for_spawn,
                    elapsed = elapsed.as_secs_f64()
                );
                std::io::stdout().flush().ok();
                idx = (idx + 1) % SPINNER_CHARS.len();
            }
        });

        Self {
            handle: Some(handle),
            msg,
            start: tokio::time::Instant::now(),
        }
    }

    pub fn stop(&mut self) {
        if let Some(h) = self.handle.take() {
            h.abort();
        }
        let goto = cursor_to_col1();
        print!("{ERASE_LINE}{goto}");
        std::io::stdout().flush().ok();
    }

    pub fn finish(&mut self, completion: &str) {
        if let Some(h) = self.handle.take() {
            h.abort();
        }
        let elapsed = self.start.elapsed();
        let goto = cursor_to_col1();
        print!("{ERASE_LINE}{goto}");
        println!("{} {:.1}s {}", self.msg, elapsed.as_secs_f64(), completion);
        std::io::stdout().flush().ok();
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.msg
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        self.stop();
    }
}
