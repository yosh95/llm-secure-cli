use crate::cli::interactive::completion::ChatCompleter;
use crate::cli::ui;
use crate::consts::HISTORY_LOG_PATH;
use crate::core::session::ChatSession;
use crate::llm::models::DataSource;
use colored::*;
use rustyline::error::ReadlineError;
use rustyline::history::FileHistory;
use rustyline::{Cmd, Editor, KeyCode, KeyEvent, Modifiers};
use serde_json;
use std::sync::{Arc, Mutex};
use tokio;

impl ChatSession {
    pub async fn run(
        &mut self,
        initial_data: Option<Vec<DataSource>>,
        _sources: Option<Vec<String>>,
    ) {
        let data = initial_data.unwrap_or_default();
        let is_stdout = self.client.get_state().stdout;

        if !data.is_empty() {
            if self.intent.is_empty()
                && let Some(DataSource {
                    content: serde_json::Value::String(s),
                    ..
                }) = data.first()
            {
                self.intent = s.clone();
                crate::utils::chat_logger::log_chat(
                    &self.ctx.config_manager,
                    &crate::llm::models::Role::User,
                    s,
                    None,
                );
            }

            match self.process_and_print(data).await {
                Ok(_) => {
                    if is_stdout {
                        return;
                    }
                }
                Err(e) => {
                    ui::report_error(&format!("Error: {}", e));
                    if is_stdout {
                        return;
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

        let mut rl = Editor::<ChatCompleter, FileHistory>::with_config(config)
            .expect("Failed to create editor");
        rl.set_helper(Some(ChatCompleter::new(
            current_provider.clone(),
            self.ctx.clone(),
        )));

        rl.bind_sequence(KeyEvent(KeyCode::Up, Modifiers::NONE), Cmd::PreviousHistory);
        rl.bind_sequence(KeyEvent(KeyCode::Down, Modifiers::NONE), Cmd::NextHistory);
        rl.bind_sequence(KeyEvent(KeyCode::Char('j'), Modifiers::CTRL), Cmd::Newline);
        rl.bind_sequence(KeyEvent(KeyCode::Up, Modifiers::CTRL), Cmd::PreviousHistory);
        rl.bind_sequence(KeyEvent(KeyCode::Down, Modifiers::CTRL), Cmd::NextHistory);

        if let Some(parent) = HISTORY_LOG_PATH.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = rl.load_history(&*HISTORY_LOG_PATH);

        let mut next_initial_text: Option<String> = None;

        loop {
            {
                let mut cp = current_provider.lock().unwrap();
                *cp = self.client.get_state().provider.clone();
            }

            let readline = if let Some(initial) = next_initial_text.take() {
                rl.readline_with_initial("> ", (&initial, ""))
            } else {
                rl.readline("> ")
            };

            match readline {
                Ok(line) => {
                    let final_trimmed = line.trim().to_string();
                    if final_trimmed.is_empty() {
                        continue;
                    }

                    let (content, should_continue) =
                        match crate::cli::interactive::dispatcher::handle_command(
                            self,
                            &final_trimmed,
                        )
                        .await
                        {
                            crate::cli::interactive::dispatcher::CommandResult::Exit => {
                                let _ = rl.save_history(&*HISTORY_LOG_PATH);
                                // Force drop to save session anchor before exit
                                drop(rl);
                                let temp_self = std::mem::replace(
                                    self,
                                    ChatSession::new_empty(self.ctx.clone()),
                                );
                                drop(temp_self);
                                std::process::exit(0);
                            }
                            crate::cli::interactive::dispatcher::CommandResult::Handled => {
                                let _ = rl.add_history_entry(&final_trimmed);
                                (None, true)
                            }
                            crate::cli::interactive::dispatcher::CommandResult::NotACommand => {
                                (Some(final_trimmed.clone()), false)
                            }
                            crate::cli::interactive::dispatcher::CommandResult::Input(text) => {
                                next_initial_text = Some(text);
                                (None, true)
                            }
                        };

                    if should_continue {
                        continue;
                    }

                    let _ = rl.add_history_entry(&final_trimmed);
                    let final_content = content.unwrap_or_else(|| final_trimmed.to_string());

                    if self.intent.is_empty() {
                        self.intent = final_content.clone();
                    }

                    crate::utils::chat_logger::log_chat(
                        &self.ctx.config_manager,
                        &crate::llm::models::Role::User,
                        &final_content,
                        None,
                    );

                    // Persist history after each entry so it survives
                    // SIGKILL / OOM kills on Android where the process
                    // may be terminated before the deferred save_history
                    // on normal exit can run.
                    let _ = rl.save_history(&*HISTORY_LOG_PATH);

                    let mut data = std::mem::take(&mut self.pending_data);
                    data.push(DataSource {
                        content: serde_json::Value::String(final_content),
                        content_type: "text/plain".to_string(),
                        is_file_or_url: false,
                        metadata: std::collections::HashMap::new(),
                    });

                    let mut process_future = Box::pin(self.process_and_print(data));
                    match tokio::select! {
                        res = &mut process_future => res,
                        _ = tokio::signal::ctrl_c() => {
                            drop(process_future);
                            println!("\n^C - Interrupted. Returning to prompt...");
                            self.handle_interruption();
                            Ok(())
                        }
                    } {
                        Ok(_) => {}
                        Err(e) => ui::report_error(&format!("Error: {}", e)),
                    }
                }
                Err(ReadlineError::Interrupted) => {
                    println!("CTRL-C");
                    break;
                }
                Err(ReadlineError::Eof) => {
                    println!("CTRL-D");
                    let _ = rl.save_history(&*HISTORY_LOG_PATH);
                    drop(rl);
                    // Ensure the session anchor is created by dropping self
                    let temp_self =
                        std::mem::replace(self, ChatSession::new_empty(self.ctx.clone()));
                    drop(temp_self);
                    std::process::exit(0);
                }
                Err(err) => {
                    ui::report_error(&format!("Error: {:?}", err));
                    break;
                }
            }
        }
        let _ = rl.save_history(&*HISTORY_LOG_PATH);
    }
}
