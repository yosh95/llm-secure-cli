use crate::cli::interactive::completion::ChatCompleter;
use crate::cli::ui;
use crate::consts::history_log_path;
use crate::core::session::ActiveSession;
use crate::llm::models::DataSource;
use rustyline::error::ReadlineError;
use rustyline::history::{FileHistory, History};
use rustyline::{
    Cmd, ConditionalEventHandler, Editor, Event, EventContext, EventHandler, KeyCode, KeyEvent,
    Modifiers,
};
use serde_json;
use std::io::{BufRead, BufReader, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Attempt to restore the terminal to a sane cooked mode.
/// This is a safety net to recover from a state where rustyline left
/// the terminal in raw mode (ISIG disabled), which would cause Ctrl+C
/// to not generate SIGINT.
fn restore_terminal() {
    #[cfg(unix)]
    {
        // Reset terminal to sane settings via `stty`.
        // This is the most reliable cross-platform way to restore terminal state
        // without depending on specific terminal libraries.
        let _ = std::process::Command::new("stty").args(["sane"]).status();

        // Also try to re-enable ISIG explicitly via `stty icanon isig`.
        let _ = std::process::Command::new("stty")
            .args(["icanon", "isig"])
            .status();
    }
}

/// Load history with a fallback for platforms where rustyline's native
/// `FileHistory::load` (which uses `flock`) may fail (e.g., Termux/Android).
fn load_history_robust(rl: &mut Editor<ChatCompleter, FileHistory>, path: &std::path::Path) {
    // Try rustyline's native load first
    if rl.load_history(path).is_ok() {
        return;
    }
    // Fallback: read the file manually. rustyline writes in V2 format
    // (#V2 header line, then each line has \n and \\ escaped).
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return,
    };
    let reader = BufReader::new(file);
    let mut lines = reader.lines();
    // Check for V2 header
    let mut v2 = false;
    if let Some(Ok(first)) = lines.next() {
        if first == "#V2" {
            v2 = true;
        } else if let Err(e) = rl.add_history_entry(&first) {
            tracing::warn!("Failed to add history entry: {}", e);
        }
    }
    for line in lines {
        let mut line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        if line.is_empty() {
            continue;
        }
        if v2 {
            // Unescape \n → newline, \\ → backslash
            let mut unescaped = String::with_capacity(line.len());
            let mut chars = line.chars();
            while let Some(c) = chars.next() {
                if c == '\\' {
                    match chars.next() {
                        Some('n') => unescaped.push('\n'),
                        Some('\\') => unescaped.push('\\'),
                        Some(other) => {
                            unescaped.push('\\');
                            unescaped.push(other);
                        }
                        None => unescaped.push('\\'),
                    }
                } else {
                    unescaped.push(c);
                }
            }
            line = unescaped;
        }
        if let Err(e) = rl.add_history_entry(&line) {
            tracing::warn!("Failed to add history entry: {}", e);
        }
    }
}

/// Save history with a fallback for platforms where rustyline's native
/// `FileHistory::save` (which uses `flock` and `umask` via the `nix` crate)
/// may fail (e.g., Termux/Android, some FUSE filesystems).
#[allow(dead_code)]
fn save_history_robust(rl: &mut Editor<ChatCompleter, FileHistory>, path: &std::path::Path) {
    // Try rustyline's native save first
    if rl.save_history(path).is_ok() {
        return;
    }
    // Fallback: write the history using plain file I/O (no flock, no umask).
    // This mirrors rustyline's V2 format.
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        tracing::error!("Failed to create directory {:?}: {}", parent, e);
    }
    // Collect all entries first so we don't hold a borrow on `rl` while
    // doing I/O.
    let entries: Vec<String> = {
        let history = rl.history();
        if history.is_empty() {
            return;
        }
        history.iter().cloned().collect()
    };
    let mut file = match std::fs::File::create(path) {
        Ok(f) => f,
        Err(_) => return,
    };
    if let Err(e) = file.write_all(b"#V2\n") {
        tracing::error!("Failed to write history V2 header: {}", e);
    }
    for entry in &entries {
        let escaped = entry.replace('\\', "\\\\").replace('\n', "\\n");
        if let Err(e) = writeln!(file, "{escaped}") {
            tracing::warn!("Failed to write history entry: {}", e);
        }
    }
}

/// A conditional event handler that detects F2 and sets the
/// `edit_pending` flag so the main loop can open an external editor.
struct EditOnF2 {
    edit_pending: Arc<AtomicBool>,
}

impl ConditionalEventHandler for EditOnF2 {
    fn handle(
        &self,
        _evt: &Event,
        _n: rustyline::RepeatCount,
        _positive: bool,
        _ctx: &EventContext,
    ) -> Option<Cmd> {
        // Mark that the next accepted input should be opened in an external editor.
        self.edit_pending.store(true, Ordering::SeqCst);
        // Accept the current line (same as pressing Enter).
        Some(Cmd::AcceptLine)
    }
}

/// Async version of save_history_robust that offloads the file I/O
/// to a blocking thread pool, avoiding stalls on the tokio runtime.
async fn save_history_async(rl: &mut Editor<ChatCompleter, FileHistory>, path: &std::path::Path) {
    // Collect entries synchronously (fast, memcpy only)
    let entries: Vec<String> = {
        let history = rl.history();
        if history.is_empty() {
            return;
        }
        history.iter().cloned().collect()
    };

    let path = path.to_path_buf();

    tokio::task::spawn_blocking(move || {
        // Ensure parent directory exists
        if let Some(parent) = path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            tracing::error!("Failed to create directory {:?}: {}", parent, e);
            return;
        }

        let mut file = match std::fs::File::create(&path) {
            Ok(f) => f,
            Err(e) => {
                tracing::error!("Failed to create history file {:?}: {}", path, e);
                return;
            }
        };

        if let Err(e) = file.write_all(b"#V2\n") {
            tracing::error!("Failed to write history V2 header: {}", e);
            return;
        }

        for entry in &entries {
            let escaped = entry.replace('\\', "\\\\").replace('\n', "\\n");
            if let Err(e) = writeln!(file, "{escaped}") {
                tracing::warn!("Failed to write history entry: {}", e);
                return;
            }
        }
    })
    .await
    .ok();
}
impl ActiveSession {
    pub async fn run(
        &mut self,
        initial_data: Option<Vec<DataSource>>,
        _sources: Option<Vec<String>>,
    ) {
        let is_stdout = self.client.get_state().stdout;

        if let Some(mut data) = initial_data
            && !data.is_empty()
        {
            // Check if there's any actual text prompt from the user in the initial data
            let has_text_prompt = data.iter().any(|d| {
                d.content.as_str().is_some_and(|s| !s.trim().is_empty()) && !d.is_file_or_url
            });

            // If only files were provided without a text prompt, insert a default prompt
            // to satisfy LLM requirements (e.g., Gemini requires at least 1 text token).
            if !has_text_prompt {
                data.push(DataSource {
                    content: serde_json::Value::String(
                        "The following content has been extracted and provided as context for this session. Please analyze this information and provide a summary or overview of its key points.".to_string(),
                    ),
                    content_type: "text/plain".to_string(),
                    is_file_or_url: false,
                    metadata: std::collections::HashMap::new(),
                });
            }

            if self.intent.is_empty() {
                // Set a default intent for the session
                self.intent = data
                    .first()
                    .and_then(|d| d.metadata.get("filename"))
                    .and_then(|v| v.as_str())
                    .map_or_else(
                        || "Initial file analysis".to_string(),
                        |f| format!("Analysis of {f}"),
                    );
            }

            let model_is_empty = self.client.get_state().model.is_empty();

            if model_is_empty {
                ui::report_error("Model is not set. Cannot process initial data.");
                if is_stdout {
                    return;
                }
            } else {
                match self.process_and_print(data).await {
                    Ok(()) => {
                        if is_stdout {
                            return;
                        }
                    }
                    Err(e) => {
                        ui::report_error(&format!("Error: {e}"));
                        if is_stdout {
                            return;
                        }
                    }
                }
            }
        }

        if is_stdout {
            return;
        }

        println!("Use Ctrl+C or /q to exit, /h for help.");

        let current_provider = Arc::new(Mutex::new(self.client.get_state().provider.clone()));
        let config = rustyline::Config::builder()
            .history_ignore_space(true)
            .completion_type(rustyline::CompletionType::List)
            .edit_mode(rustyline::EditMode::Emacs)
            .bracketed_paste(true)
            .build();

        let mut rl = match Editor::<ChatCompleter, FileHistory>::with_config(config) {
            Ok(e) => e,
            Err(e) => {
                ui::report_error(&format!("Failed to create editor: {e}"));
                return;
            }
        };
        rl.set_helper(Some(ChatCompleter::new(
            current_provider.clone(),
            self.ctx.clone(),
        )));

        rl.bind_sequence(KeyEvent(KeyCode::Up, Modifiers::NONE), Cmd::PreviousHistory);
        rl.bind_sequence(KeyEvent(KeyCode::Down, Modifiers::NONE), Cmd::NextHistory);
        rl.bind_sequence(KeyEvent(KeyCode::Char('j'), Modifiers::CTRL), Cmd::Newline);
        rl.bind_sequence(KeyEvent(KeyCode::Up, Modifiers::CTRL), Cmd::PreviousHistory);
        rl.bind_sequence(KeyEvent(KeyCode::Down, Modifiers::CTRL), Cmd::NextHistory);

        // Quick navigation: Ctrl + Home/End for navigating long prompts
        rl.bind_sequence(
            KeyEvent(KeyCode::Home, Modifiers::CTRL),
            Cmd::Move(rustyline::Movement::BeginningOfBuffer),
        );
        rl.bind_sequence(
            KeyEvent(KeyCode::End, Modifiers::CTRL),
            Cmd::Move(rustyline::Movement::EndOfBuffer),
        );

        // --- F2: open external editor for current prompt (use F2 to avoid Emacs Ctrl+E conflict) ---
        let edit_pending = Arc::new(AtomicBool::new(false));
        let f2_handler = EditOnF2 {
            edit_pending: edit_pending.clone(),
        };
        rl.bind_sequence(
            KeyEvent(KeyCode::F(2), Modifiers::NONE),
            EventHandler::Conditional(Box::new(f2_handler)),
        );

        let h_path = history_log_path();
        if let Some(parent) = h_path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            tracing::error!("Failed to create directory {:?}: {}", parent, e);
        }
        load_history_robust(&mut rl, &h_path);

        let mut next_initial_text: Option<String> = None;

        loop {
            {
                let mut cp = match current_provider.lock() {
                    Ok(guard) => guard,
                    Err(e) => {
                        ui::report_error(&format!("Lock poisoned: {e}"));
                        break;
                    }
                };
                *cp = self.client.get_state().provider.clone();
            }

            let readline = if let Some(initial) = next_initial_text.take() {
                rl.readline_with_initial("> ", (&initial, ""))
            } else {
                rl.readline("> ")
            };

            match readline {
                Ok(line) => {
                    let raw_line = line.trim().to_string();

                    // Check F2 BEFORE the empty-line check so F2 works on
                    // an empty prompt (just open a blank editor and go).
                    if edit_pending.swap(false, Ordering::SeqCst) {
                        match ui::open_external_editor(&raw_line) {
                            Ok(edited) => {
                                let trimmed = edited.trim().to_string();
                                if trimmed.is_empty() {
                                    ui::report_warning("Empty input from editor, skipping.");
                                    continue;
                                }
                                // Set the edited content as the initial value for the next readline,
                                // so the user can review/edit it before sending.
                                next_initial_text = Some(trimmed);
                                continue;
                            }
                            Err(e) => {
                                ui::report_error(&format!("Failed to open editor: {e}"));
                                continue;
                            }
                        }
                    }

                    if raw_line.is_empty() {
                        continue;
                    }

                    let final_trimmed = raw_line;
                    if final_trimmed.is_empty() {
                        continue;
                    }

                    match crate::cli::interactive::dispatcher::handle_command(self, &final_trimmed)
                        .await
                    {
                        crate::cli::interactive::dispatcher::CommandResult::Exit => {
                            save_history_async(&mut rl, &history_log_path()).await;
                            // Auto-save the session before exiting.
                            crate::utils::session_store::auto_save_async(self).await;
                            // Drop rustyline editor and return to let
                            // ChatSession Drop run naturally (saves Merkle anchor).
                            drop(rl);
                            return;
                        }
                        crate::cli::interactive::dispatcher::CommandResult::Handled => {
                            if let Err(e) = rl.add_history_entry(&final_trimmed) {
                                tracing::warn!("Failed to add history entry: {}", e);
                            }
                            save_history_async(&mut rl, &history_log_path()).await;
                            continue;
                        }
                        crate::cli::interactive::dispatcher::CommandResult::NotACommand => {}
                        crate::cli::interactive::dispatcher::CommandResult::Input(text) => {
                            if let Err(e) = rl.add_history_entry(&final_trimmed) {
                                tracing::warn!("Failed to add history entry: {}", e);
                            }
                            next_initial_text = Some(text);
                            continue;
                        }
                    }

                    if let Err(e) = rl.add_history_entry(&final_trimmed) {
                        tracing::warn!("Failed to add history entry: {}", e);
                    }
                    let final_content = final_trimmed.clone();

                    if self.intent.is_empty() {
                        self.intent = final_content.clone();
                    }

                    // Persist history after each entry so it survives
                    // SIGKILL / OOM kills on Android where the process
                    // may be terminated before the deferred save_history
                    // on normal exit can run.
                    save_history_async(&mut rl, &history_log_path()).await;

                    let mut data = std::mem::take(&mut self.pending_data);
                    data.push(DataSource {
                        content: serde_json::Value::String(final_content),
                        content_type: "text/plain".to_string(),
                        is_file_or_url: false,
                        metadata: std::collections::HashMap::new(),
                    });

                    let model_is_empty = self.client.get_state().model.is_empty();

                    if model_is_empty {
                        ui::report_error(
                            "Model is not set. Please use /model <model_name> to set a model before sending requests.",
                        );
                        // Put data back to pending
                        self.pending_data = data;
                        continue;
                    }

                    if let Err(e) = self.process_and_print(data).await {
                        ui::report_error(&format!("Error: {e}"));
                    }
                }
                Err(ReadlineError::Interrupted) => {
                    // Restore terminal to cooked mode to ensure future
                    // Ctrl+C signals work correctly (rustyline may leave
                    // the terminal in raw mode on interrupt).
                    restore_terminal();
                    println!("CTRL-C");
                    // Auto-save the session before exiting on Ctrl+C.
                    crate::utils::session_store::auto_save_async(self).await;
                    break;
                }
                Err(ReadlineError::Eof) => {
                    println!("CTRL-D");
                    save_history_async(&mut rl, &history_log_path()).await;
                    // Auto-save the session before exiting on Ctrl+D.
                    crate::utils::session_store::auto_save_async(self).await;
                    drop(rl);
                    // Return to let ChatSession Drop run naturally
                    // (saves Merkle anchor and cleanup).
                    return;
                }
                Err(err) => {
                    if matches!(&err, ReadlineError::Io(e) if e.kind() == std::io::ErrorKind::WouldBlock)
                    {
                        // Terminal settings may be corrupted; restore and retry
                        eprintln!("\rWARNING WouldBlock - terminal busy, resetting...");
                        let _ = std::process::Command::new("stty").args(["sane"]).status();
                        continue;
                    }
                    ui::report_error(&format!("Error: {err:?}"));
                    break;
                }
            }
        }
        // Auto-save the session after the main loop ends (e.g. Ctrl+C).
        crate::utils::session_store::auto_save_async(self).await;
        save_history_async(&mut rl, &history_log_path()).await;
    }
}
