use rustyline::completion::{Completer, FilenameCompleter, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
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
                "/d",
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
                                    matches.push(Pair {
                                        display: alias.clone(),
                                        replacement: alias.clone(),
                                    });
                                }
                            }
                        } else {
                            // Fallback to all models if provider not found
                            for p_cfg in config.providers.values() {
                                for alias in p_cfg.models.keys() {
                                    if alias.starts_with(arg_prefix) {
                                        matches.push(Pair {
                                            display: alias.clone(),
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
                    "/tools" => {
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

impl Highlighter for ChatCompleter {}

impl Validator for ChatCompleter {}

impl Helper for ChatCompleter {}
