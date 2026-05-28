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
    /// The spinner ticks every 80 ms, overwriting the same line with `\r`,
    /// showing the elapsed time in seconds.
    /// Call [`finish`](Self::finish) or [`stop`](Self::stop) when done.
    pub fn start(msg: &str) -> Self {
        let msg = msg.to_string();
        let msg_for_spawn = msg.clone();
        // Print the first frame immediately with spinner char and elapsed time,
        // so the layout is consistent from the start (no rightward shift later).
        print!("\r{} {} (0.0s)", SPIN_CHARS[0], msg);
        std::io::stdout().flush().ok();

        let handle = tokio::spawn(async move {
            let start = tokio::time::Instant::now();
            let mut idx: usize = 1; // index 0 already shown, start from next
            loop {
                tokio::time::sleep(Duration::from_millis(80)).await;
                let elapsed = start.elapsed();
                print!(
                    "\r{} {} ({:.1}s)",
                    SPIN_CHARS[idx],
                    msg_for_spawn,
                    elapsed.as_secs_f64()
                );
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
        // Clear the line (generous width to cover elapsed time display)
        let width = self.msg.len() + 20;
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
        // Clear the line first to avoid leftover characters from elapsed time display
        let width = self.msg.len() + 30;
        print!("\r{}\r", " ".repeat(width));
        println!("{} {}", self.msg, completion);
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
