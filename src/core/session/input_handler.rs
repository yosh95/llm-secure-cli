use crate::cli::interactive::completion::ChatCompleter;
use crate::cli::ui;
use crate::consts::history_log_path;
use crate::core::session::ActiveSession;
use crate::llm::models::DataSource;
use colored::Colorize;
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

/// A conditional event handler that detects Ctrl+E and sets the
/// `edit_pending` flag so the main loop can open an external editor.
struct EditOnCtrlE {
    edit_pending: Arc<AtomicBool>,
}

impl ConditionalEventHandler for EditOnCtrlE {
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

        println!("{}", "Use Ctrl+C or /q to exit, /h for help.".dimmed());

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

        // --- Ctrl+E: open external editor for current prompt ---
        let edit_pending = Arc::new(AtomicBool::new(false));
        let ctrl_e_handler = EditOnCtrlE {
            edit_pending: edit_pending.clone(),
        };
        rl.bind_sequence(
            KeyEvent(KeyCode::Char('e'), Modifiers::CTRL),
            EventHandler::Conditional(Box::new(ctrl_e_handler)),
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
                    if raw_line.is_empty() {
                        continue;
                    }

                    // Check if we should open the external editor for this input
                    // (triggered by Ctrl+E).
                    if edit_pending.swap(false, Ordering::SeqCst) {
                        match ui::open_external_editor(&raw_line) {
                            Ok(edited) => {
                                let trimmed = edited.trim().to_string();
                                if trimmed.is_empty() {
                                    ui::report_warning("Empty input from editor, skipping.");
                                    continue;
                                }
                                // Set the edited content as the initial text for the
                                // next prompt so the user can review / continue editing.
                                next_initial_text = Some(trimmed);
                                continue;
                            }
                            Err(e) => {
                                ui::report_error(&format!("Failed to open editor: {e}"));
                                continue;
                            }
                        }
                    }

                    let final_trimmed = raw_line;
                    if final_trimmed.is_empty() {
                        continue;
                    }

                    match crate::cli::interactive::dispatcher::handle_command(self, &final_trimmed)
                        .await
                    {
                        crate::cli::interactive::dispatcher::CommandResult::Exit => {
                            save_history_robust(&mut rl, &history_log_path());
                            // Drop rustyline editor and return to let
                            // ChatSession Drop run naturally (saves Merkle anchor).
                            drop(rl);
                            return;
                        }
                        crate::cli::interactive::dispatcher::CommandResult::Handled => {
                            if let Err(e) = rl.add_history_entry(&final_trimmed) {
                                tracing::warn!("Failed to add history entry: {}", e);
                            }
                            save_history_robust(&mut rl, &history_log_path());
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
                    save_history_robust(&mut rl, &history_log_path());

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
                    println!("CTRL-C");
                    break;
                }
                Err(ReadlineError::Eof) => {
                    println!("CTRL-D");
                    save_history_robust(&mut rl, &history_log_path());
                    drop(rl);
                    // Return to let ChatSession Drop run naturally
                    // (saves Merkle anchor and cleanup).
                    return;
                }
                Err(err) => {
                    ui::report_error(&format!("Error: {err:?}"));
                    break;
                }
            }
        }
        save_history_robust(&mut rl, &history_log_path());
    }
}
