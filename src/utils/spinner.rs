use std::io::Write;
use std::time::Duration;

/// A minimal spinner that displays an animated indicator while a task runs.
///
/// Uses only `tokio::time` and `std::io` — no external dependencies.
///
/// # Example
///
/// ```ignore
/// let spin = Spinner::start("Loading ...");
/// do_work().await;
/// spin.finish("done");  // → "Loading ... done"
/// ```
///
/// On early returns (e.g. `?` operator), `Drop` automatically cleans up the line.
pub struct Spinner {
    handle: Option<tokio::task::JoinHandle<()>>,
    msg: String,
}

const SPIN_CHARS: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

impl Spinner {
    /// Start a new spinner with the given message.
    ///
    /// The spinner ticks every 80 ms, overwriting the same line with `\r`.
    /// Call [`finish`](Self::finish) or [`stop`](Self::stop) when done.
    pub fn start(msg: &str) -> Self {
        let msg = msg.to_string();
        let msg_for_spawn = msg.clone();
        // Initial print (no spinner char yet)
        print!("{}", msg);
        std::io::stdout().flush().ok();

        let handle = tokio::spawn(async move {
            let mut idx: usize = 0;
            loop {
                tokio::time::sleep(Duration::from_millis(80)).await;
                print!("\r{} {}", SPIN_CHARS[idx], msg_for_spawn);
                std::io::stdout().flush().ok();
                idx = (idx + 1) % SPIN_CHARS.len();
            }
        });

        Self {
            handle: Some(handle),
            msg,
        }
    }

    /// Stop the spinner and clear the current line completely.
    pub fn stop(&mut self) {
        if let Some(h) = self.handle.take() {
            h.abort();
        }
        // Clear the line
        let width = self.msg.len() + 3; // spinner char + space + msg
        print!("\r{}\r", " ".repeat(width));
        std::io::stdout().flush().ok();
    }

    /// Stop the spinner and write a completion message on the same line.
    ///
    /// # Example
    ///
    /// ```ignore
    /// spin.finish("done");  // prints "Loading ... done" on the spinner line
    /// ```
    pub fn finish(&mut self, completion: &str) {
        if let Some(h) = self.handle.take() {
            h.abort();
        }
        print!("\r{} {}\n", self.msg, completion);
        std::io::stdout().flush().ok();
    }

    /// Borrow the spinner message.
    pub fn message(&self) -> &str {
        &self.msg
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        self.stop();
    }
}
