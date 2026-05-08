use crate::core::context::AppContext;
use rustyline::Context;
use rustyline::Helper;
use rustyline::completion::{Completer, FilenameCompleter, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::{CmdKind, Highlighter};
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use std::sync::{Arc, Mutex};

pub struct ChatCompleter {
    file_completer: FilenameCompleter,
    commands: Vec<&'static str>,
    pub current_provider: Arc<Mutex<String>>,
    pub ctx: Arc<AppContext>,
}

impl ChatCompleter {
    pub fn new(current_provider: Arc<Mutex<String>>, ctx: Arc<AppContext>) -> Self {
        Self {
            file_completer: FilenameCompleter::new(),
            commands: vec![
                "/help",
                "/h",
                "/quit",
                "/q",
                "/edit",
                "/e",
                "/clear",
                "/c",
                "/info",
                "/i",
                "/system",
                "/raw",
                "/dump",
                "/save",
                "/load",
                "/attach",
                "/tools",
                "/model",
                "/models",
                "/m",
                "/vmodel",
                "/vm",
                "/provider",
                "/p",
                "/vprovider",
                "/vp",
                "/checkpoint",
                "/cp",
                "/edit_history",
                "/eh",
            ],
            current_provider,
            ctx,
        }
    }
}

impl Completer for ChatCompleter {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        ctx: &Context<'_>,
    ) -> Result<(usize, Vec<Pair>), ReadlineError> {
        if line.starts_with('/') {
            let parts: Vec<&str> = line[..pos].splitn(2, ' ').collect();
            if parts.len() == 1 {
                // Completing the command itself
                let mut matches = Vec::new();
                for cmd in &self.commands {
                    if cmd.starts_with(line) {
                        matches.push(Pair {
                            display: cmd.to_string(),
                            replacement: cmd.to_string(),
                        });
                    }
                }
                return Ok((0, matches));
            } else {
                // Completing arguments
                let cmd = parts[0];
                let arg_prefix = parts[1];
                let start = cmd.len() + 1;

                match cmd {
                    "/load" | "/save" | "/attach" | "/edit" | "/e" => {
                        return self.file_completer.complete(line, pos, ctx);
                    }
                    "/provider" | "/p" | "/vprovider" | "/vp" => {
                        let providers = self.ctx.config_manager.get_active_providers();
                        let mut matches = Vec::new();
                        for p in providers {
                            if p.starts_with(arg_prefix) {
                                matches.push(Pair {
                                    display: p.clone(),
                                    replacement: p,
                                });
                            }
                        }
                        return Ok((start, matches));
                    }
                    "/model" | "/m" | "/models" | "/vmodel" | "/vm" => {
                        let models_map = self.ctx.config_manager.get_cached_models_sync();
                        let mut matches = Vec::new();
                        let current_p = self
                            .current_provider
                            .lock()
                            .expect("mutex lock failed")
                            .clone();

                        // Add aliases to completions
                        if let Ok(state) = self.ctx.config_manager.get_state() {
                            for alias in state.model_aliases.keys() {
                                if alias.starts_with(arg_prefix) {
                                    matches.push(Pair {
                                        display: format!("{} (alias)", alias),
                                        replacement: alias.clone(),
                                    });
                                }
                            }
                        }

                        // Suggest models for the CURRENT provider directly
                        if let Some(models) = models_map.get(&current_p) {
                            for model in models {
                                if model.starts_with(arg_prefix) {
                                    matches.push(Pair {
                                        display: model.clone(),
                                        replacement: model.clone(),
                                    });
                                }
                            }
                        }

                        // If user has typed "provider/", suggest models for that provider (keep for flexibility)
                        if let Some((p_prefix, m_prefix)) = arg_prefix.split_once('/') {
                            if let Some(models) = models_map.get(p_prefix) {
                                for model in models {
                                    if model.starts_with(m_prefix) {
                                        matches.push(Pair {
                                            display: format!("{}/{}", p_prefix, model),
                                            replacement: format!("{}/{}", p_prefix, model),
                                        });
                                    }
                                }
                            }
                        } else {
                            // Suggest "provider/" for all active providers if it's not the current one
                            let providers = self.ctx.config_manager.get_active_providers();
                            for p in providers {
                                if p != current_p && p.starts_with(arg_prefix) {
                                    matches.push(Pair {
                                        display: format!("{}/", p),
                                        replacement: format!("{}/", p),
                                    });
                                }
                            }
                        }

                        matches.sort_by(|a, b| a.display.cmp(&b.display));
                        matches.dedup_by(|a, b| a.display == b.display);
                        return Ok((start, matches));
                    }
                    "/tools" | "/system" => {
                        let mut matches = Vec::new();
                        for opt in &["on", "off"] {
                            if opt.starts_with(arg_prefix) {
                                matches.push(Pair {
                                    display: opt.to_string(),
                                    replacement: opt.to_string(),
                                });
                            }
                        }
                        return Ok((start, matches));
                    }
                    _ => {}
                }
            }
        }
        Ok((0, Vec::new()))
    }
}

impl Hinter for ChatCompleter {
    type Hint = String;
}

impl Highlighter for ChatCompleter {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> std::borrow::Cow<'l, str> {
        if line.starts_with('/') {
            let parts: Vec<&str> = line.splitn(2, ' ').collect();
            let cmd = parts[0];
            if self.commands.contains(&cmd) {
                let mut highlighted = line.to_string();
                // Use ANSI codes directly to avoid conflicts with rustyline's own handling
                // \x1b[1;36m is bold cyan, \x1b[0m is reset
                let colored_cmd = format!("\x1b[1;34m{}\x1b[0m", cmd);
                highlighted = highlighted.replacen(cmd, &colored_cmd, 1);
                return std::borrow::Cow::Owned(highlighted);
            }
        }

        // Basic highlighting for markdown-style code blocks in input
        if line.contains("```") {
            let mut highlighted = line.to_string();
            highlighted = highlighted.replace("```", "\x1b[1;33m```\x1b[0m");
            return std::borrow::Cow::Owned(highlighted);
        }

        std::borrow::Cow::Borrowed(line)
    }

    fn highlight_char(&self, _line: &str, _pos: usize, _forced: CmdKind) -> bool {
        true
    }
}

impl Validator for ChatCompleter {
    fn validate(
        &self,
        ctx: &mut rustyline::validate::ValidationContext,
    ) -> rustyline::Result<rustyline::validate::ValidationResult> {
        let input = ctx.input();

        // Simple check for unclosed code blocks
        let backtick_count = input.matches("```").count();
        if !backtick_count.is_multiple_of(2) {
            return Ok(rustyline::validate::ValidationResult::Incomplete);
        }

        // Allow explicit newline continuation with \
        if input.ends_with('\\') {
            return Ok(rustyline::validate::ValidationResult::Incomplete);
        }

        Ok(rustyline::validate::ValidationResult::Valid(None))
    }
}

impl Helper for ChatCompleter {}
