use std::io::Write;
use std::time::Duration;

/// A minimal elapsed-time display that shows a message with ticking seconds.
///
/// Uses only `tokio::time` and `std::io` \u2014 no external dependencies.
///
/// # Example
///
/// ```text
/// let spin = Spinner::start("Loading \u2026");
/// do_work().await;
/// spin.finish("done");  // \u2192 "Loading ... 3.2s done"
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

/// Reserve space for: space + "XX.Xs" (6 chars)
const RESERVED_COLS: u16 = 7;

/// Truncate `msg` at the **beginning** so the tail (model name) is preserved.
/// e.g. "openai/gpt-4o-mini-longname..." \u2192 "\u2026/gpt-4o-mini-longname"
fn truncate_msg(msg: &mut String, term_width: u16) {
    let max_msg_len = term_width.saturating_sub(RESERVED_COLS) as usize;
    if msg.len() > max_msg_len && max_msg_len > 3 {
        let suffix: String = msg.chars().skip(msg.len() - max_msg_len + 1).collect();
        *msg = "\u{2026}".to_string() + &suffix;
    }
}

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
            "{erase}{goto}{msg} 0.0s",
            erase = ERASE_LINE,
            goto = goto,
            msg = msg
        );
        std::io::stdout().flush().ok();

        let handle = tokio::spawn(async move {
            let start = tokio::time::Instant::now();
            let goto = cursor_to_col1();
            loop {
                tokio::time::sleep(Duration::from_millis(200)).await;
                let elapsed = start.elapsed();
                print!(
                    "{erase}{goto}{msg} {elapsed:.1}s",
                    erase = ERASE_LINE,
                    goto = goto,
                    msg = msg_for_spawn,
                    elapsed = elapsed.as_secs_f64()
                );
                std::io::stdout().flush().ok();
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
