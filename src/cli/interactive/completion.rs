use rustyline::completion::{Completer, FilenameCompleter, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::{CmdKind, Highlighter};
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::Context;
use rustyline::Helper;
use std::sync::{Arc, Mutex};

pub struct ChatCompleter {
    file_completer: FilenameCompleter,
    commands: Vec<&'static str>,
    pub current_provider: Arc<Mutex<String>>,
}

impl ChatCompleter {
    pub fn new(current_provider: Arc<Mutex<String>>) -> Self {
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
                "/debug",
                "/system",
                "/raw",
                "/dump",
                "/save",
                "/load",
                "/attach",
                "/tools",
                "/model",
                "/m",
                "/provider",
                "/p",
                "/checkpoint",
                "/cp",
                "/reload",
            ],
            current_provider,
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
                    "/provider" | "/p" => {
                        let providers = crate::config::CONFIG_MANAGER.get_active_providers();
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
                    "/model" | "/m" => {
                        let provider = self.current_provider.lock().unwrap().clone();
                        let config = crate::config::CONFIG_MANAGER.get_config();
                        let mut matches = Vec::new();

                        if let Some(p_cfg) = config.providers.get(&provider) {
                            for alias in p_cfg.models.keys() {
                                if alias.starts_with(arg_prefix) {
                                    let model_config = crate::config::CONFIG_MANAGER
                                        .get_model_config(&provider, alias);
                                    let actual_model = model_config
                                        .get("model")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or(alias);
                                    let display = if actual_model != alias {
                                        format!("{} ({})", alias, actual_model)
                                    } else {
                                        alias.clone()
                                    };
                                    matches.push(Pair {
                                        display,
                                        replacement: alias.clone(),
                                    });
                                }
                            }
                        } else {
                            // Fallback to all models if provider not found
                            for (p_name, p_cfg) in &config.providers {
                                for alias in p_cfg.models.keys() {
                                    if alias.starts_with(arg_prefix) {
                                        let model_config = crate::config::CONFIG_MANAGER
                                            .get_model_config(p_name, alias);
                                        let actual_model = model_config
                                            .get("model")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or(alias);
                                        let display = if actual_model != alias {
                                            format!("{} ({})", alias, actual_model)
                                        } else {
                                            alias.clone()
                                        };
                                        matches.push(Pair {
                                            display,
                                            replacement: alias.clone(),
                                        });
                                    }
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
