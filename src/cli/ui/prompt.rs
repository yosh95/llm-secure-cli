use colored::Colorize;
use std::io::Read;

#[derive(Debug, PartialEq)]
pub enum ConfirmResult {
    Yes,
    No,
    Feedback(String),
}

/// Whether a confirmation prompt accepts free-text feedback or is Yes/No only.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PromptMode {
    /// Input other than Y/N is treated as free-text feedback (for LLM chat sessions).
    WithFeedback,
    /// Only Y/N is accepted; anything else re-prompts.
    YesNoOnly,
}

/// Global auto-approve flag.
/// Set from the `auto_approve` field in `\[security\]` section of config.toml.
/// When true, all confirmation prompts are automatically answered Yes.
/// WARNING: This bypasses user confirmation — use with extreme caution.
pub static AUTO_APPROVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Ask the user a Yes/No confirmation question with configurable prompt mode.
fn ask_confirm_with_mode(prompt: &str, mode: PromptMode) -> Option<ConfirmResult> {
    // If auto_approve is set (from config.toml [security] section), automatically
    // return Yes without prompting the user.
    if AUTO_APPROVE.load(std::sync::atomic::Ordering::Relaxed) {
        crate::cli::ui::report::report_warning(
            "auto_approve is enabled in config.toml — automatically approving without user confirmation.",
        );
        return Some(ConfirmResult::Yes);
    }

    let suffix = match mode {
        PromptMode::WithFeedback => " [Y/n or feedback] ",
        PromptMode::YesNoOnly => " [Y/n] ",
    };
    let y_n = format!("{prompt}{suffix}");
    match get_user_input(&y_n) {
        Some(input) => {
            let trimmed = input.trim();
            let lower = trimmed.to_lowercase();
            if lower.is_empty()
                || lower == "y"
                || lower == "yes"
                || lower == "\u{ff59}"
                || lower == "\u{ff59}\u{ff45}\u{ff53}"
            {
                Some(ConfirmResult::Yes)
            } else if lower == "n"
                || lower == "no"
                || lower == "\u{ff4e}"
                || lower == "\u{ff4e}\u{ff4f}"
            {
                Some(ConfirmResult::No)
            } else {
                match mode {
                    PromptMode::WithFeedback => {
                        println!("  {}", format!("Feedback: {trimmed}").dimmed());
                        Some(ConfirmResult::Feedback(trimmed.to_string()))
                    }
                    PromptMode::YesNoOnly => {
                        crate::cli::ui::report::report_warning(&format!(
                            "Unrecognized input '{trimmed}'. Please answer Y(es) or N(o)."
                        ));
                        ask_confirm_with_mode(prompt, mode)
                    }
                }
            }
        }
        None => None,
    }
}

/// Ask a Yes/No confirmation question that also accepts free-text feedback.
#[must_use]
pub fn ask_confirm(prompt: &str) -> Option<ConfirmResult> {
    ask_confirm_with_mode(prompt, PromptMode::WithFeedback)
}

/// Ask a Yes/No-only confirmation question (no feedback).
#[must_use]
pub fn ask_confirm_simple(prompt: &str) -> Option<ConfirmResult> {
    ask_confirm_with_mode(prompt, PromptMode::YesNoOnly)
}

pub async fn ask_confirm_async(prompt: &str) -> Option<ConfirmResult> {
    let p = prompt.to_string();
    tokio::task::spawn_blocking(move || ask_confirm(&p))
        .await
        .unwrap_or(None)
}

pub async fn ask_confirm_simple_async(prompt: &str) -> Option<ConfirmResult> {
    let p = prompt.to_string();
    tokio::task::spawn_blocking(move || ask_confirm_simple(&p))
        .await
        .unwrap_or(None)
}

#[must_use]
pub fn get_user_input(prompt: &str) -> Option<String> {
    use rustyline::DefaultEditor;
    use rustyline::error::ReadlineError;

    let mut rl = match DefaultEditor::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create editor: {e:?}");
            return None;
        }
    };
    match rl.readline(prompt) {
        Ok(line) => Some(line.trim().to_string()),
        Err(ReadlineError::Interrupted) => {
            println!("^C");
            None
        }
        Err(ReadlineError::Eof) => None,
        Err(err) => {
            if matches!(&err, ReadlineError::Io(e) if e.kind() == std::io::ErrorKind::WouldBlock) {
                // Terminal settings may be corrupted; restore and retry
                eprintln!(
                    "\r{} WouldBlock - terminal busy, resetting...",
                    "WARNING".yellow().bold()
                );
                let _ = std::process::Command::new("stty").args(["sane"]).status();
                std::thread::sleep(std::time::Duration::from_millis(100));
                return get_user_input(prompt);
            }
            eprintln!("Error: {err:?}");
            None
        }
    }
}

pub fn open_external_editor(initial_content: &str) -> anyhow::Result<String> {
    use std::io::Write;
    use std::process::Command;
    use tempfile::NamedTempFile;

    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| {
            if cfg!(windows) {
                "notepad".to_string()
            } else {
                "vi".to_string()
            }
        });

    let mut file = NamedTempFile::new()?;
    if !initial_content.is_empty() {
        file.write_all(initial_content.as_bytes())?;
    }

    let status = Command::new(editor).arg(file.path()).status()?;

    if !status.success() {
        return Err(anyhow::anyhow!("Editor exited with error status"));
    }

    let mut content = String::new();
    let mut file = std::fs::File::open(file.path())?;
    file.read_to_string(&mut content)?;

    Ok(content)
}
