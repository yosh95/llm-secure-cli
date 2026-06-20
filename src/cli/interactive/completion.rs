use crate::cli::interactive::commands;
use crate::core::context::AppContext;
use rustyline::Context;
use rustyline::Helper;
use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::{CmdKind, Highlighter};
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use std::sync::{Arc, Mutex};

pub struct ChatCompleter {
    /// Slash commands for completion — derived from the central command registry.
    commands: Vec<String>,
    pub current_provider: Arc<Mutex<String>>,
    pub ctx: Arc<AppContext>,
}

impl ChatCompleter {
    pub fn new(current_provider: Arc<Mutex<String>>, ctx: Arc<AppContext>) -> Self {
        Self {
            commands: commands::all_slash_commands(),
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
        _ctx: &Context<'_>,
    ) -> Result<(usize, Vec<Pair>), ReadlineError> {
        if line.starts_with('/') {
            let parts: Vec<&str> = line[..pos].splitn(2, ' ').collect();
            if parts.len() == 1 {
                // Completing the command itself
                let mut matches = Vec::new();
                for cmd in &self.commands {
                    if cmd.starts_with(line) {
                        matches.push(Pair {
                            display: cmd.clone(),
                            replacement: cmd.clone(),
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
                    "/session" => {
                        // Determine the subcommand being typed
                        let arg_parts: Vec<&str> = arg_prefix.split_whitespace().collect();
                        if arg_parts.is_empty() {
                            // "/session " → complete subcommands: load, delete, clear
                            let subs = ["load", "delete", "clear"];
                            let mut matches = Vec::new();
                            for sub in &subs {
                                if sub.starts_with(arg_prefix) {
                                    matches.push(Pair {
                                        display: sub.to_string(),
                                        replacement: sub.to_string(),
                                    });
                                }
                            }
                            return Ok((start, matches));
                        } else if arg_parts.len() == 1 && !arg_prefix.ends_with(' ') {
                            // "/session lo" or "/session d" → complete subcommand
                            let sub_prefix = arg_parts[0];
                            let subs = ["load", "delete", "clear"];
                            let mut matches = Vec::new();
                            for sub in &subs {
                                if sub.starts_with(sub_prefix) {
                                    matches.push(Pair {
                                        display: sub.to_string(),
                                        replacement: sub.to_string(),
                                    });
                                }
                            }
                            return Ok((start, matches));
                        } else {
                            // "/session load <prefix>" or "/session delete <prefix>"
                            let subcmd = arg_parts[0];
                            if subcmd == "clear" {
                                // no further args for clear
                                return Ok((0, Vec::new()));
                            }
                            if subcmd != "load" && subcmd != "delete" {
                                return Ok((0, Vec::new()));
                            }
                            // The session id prefix is the rest after the subcommand
                            let session_prefix = if arg_parts.len() >= 2 {
                                arg_parts[1]
                            } else {
                                ""
                            };
                            let session_start = if arg_parts.len() >= 2 {
                                start + subcmd.len() + 1 // skip subcommand and space
                            } else {
                                pos
                            };
                            // Complete session filenames (plus "last" shortcut)
                            let dir = crate::consts::sessions_dir();
                            let mut matches = Vec::new();
                            // Suggest "last" shortcut for most recent session
                            if "last".starts_with(session_prefix) {
                                matches.push(Pair {
                                    display: "last  (most recent session)".to_string(),
                                    replacement: "last".to_string(),
                                });
                            }
                            if dir.exists()
                                && let Ok(entries) = std::fs::read_dir(&dir)
                            {
                                for entry in entries.flatten() {
                                    let path = entry.path();
                                    if path.extension().and_then(|s| s.to_str()) == Some("json")
                                        && let Some(stem) =
                                            path.file_stem().and_then(|s| s.to_str())
                                        && stem.starts_with(session_prefix)
                                    {
                                        let preview = std::fs::read_to_string(&path)
                                            .ok()
                                            .and_then(|c| {
                                                serde_json::from_str::<
                                                    crate::utils::session_store::SessionFile,
                                                >(&c)
                                                .ok()
                                            })
                                            .and_then(
                                                |sf: crate::utils::session_store::SessionFile| {
                                                    use crate::llm::models::Role;
                                                    sf.conversation
                                                        .iter()
                                                        .find(|m| m.role == Role::User)
                                                        .map(|m| {
                                                            let t = m.get_text(false);
                                                            let line = t
                                                                .lines()
                                                                .next()
                                                                .unwrap_or_default();
                                                            if line.chars().count() > 36 {
                                                                format!(
                                                                    "{}...",
                                                                    line.chars()
                                                                        .take(33)
                                                                        .collect::<String>()
                                                                )
                                                            } else {
                                                                line.to_string()
                                                            }
                                                        })
                                                },
                                            );
                                        let display = if let Some(preview) = preview
                                            && !preview.is_empty()
                                        {
                                            format!("{stem}  ({preview})")
                                        } else {
                                            stem.to_string()
                                        };
                                        matches.push(Pair {
                                            display,
                                            replacement: stem.to_string(),
                                        });
                                    }
                                }
                            }
                            matches.sort_by(|a, b| a.display.cmp(&b.display));
                            return Ok((session_start, matches));
                        }
                    }
                    // /dump: no completions needed
                    "/dump" => {
                        return Ok((0, Vec::new()));
                    }

                    "/model" | "/m" => {
                        let models_map = self.ctx.config_manager.get_cached_models_sync();
                        let mut matches = Vec::new();
                        // If completing "-i" / "--info" subcommand (model info)
                        if arg_prefix == "-i"
                            || arg_prefix.starts_with("-i ")
                            || arg_prefix == "--info"
                            || arg_prefix.starts_with("--info ")
                        {
                            let flag_part = if arg_prefix.starts_with("--info") {
                                "--info"
                            } else {
                                "-i"
                            };
                            let prefix_after_flag =
                                arg_prefix.strip_prefix(flag_part).unwrap_or("");
                            let start_of_model = start + flag_part.len() + 1;
                            // After "-i " or "--info " complete with model names
                            if prefix_after_flag.starts_with(' ') && !prefix_after_flag.is_empty() {
                                let model_prefix = prefix_after_flag.trim_start();
                                // Suggest provider:model pairs
                                let mut providers: Vec<&String> = models_map.keys().collect();
                                providers.sort();
                                for p in providers {
                                    if let Some(models) = models_map.get(p) {
                                        let mut sorted_models = models.clone();
                                        sorted_models.sort();
                                        for m in sorted_models {
                                            let entry = format!("{p}:{m}");
                                            if entry.starts_with(model_prefix) {
                                                matches.push(Pair {
                                                    display: entry.clone(),
                                                    replacement: entry,
                                                });
                                            }
                                        }
                                    }
                                }
                                matches.sort_by(|a, b| a.display.cmp(&b.display));
                                matches.dedup_by(|a, b| a.display == b.display);
                                return Ok((start_of_model, matches));
                            }
                            // If prefix_after_flag is empty (e.g., arg_prefix == "-i"),
                            // complete to "-i " (with trailing space) so user can then type a model name
                            if prefix_after_flag.is_empty() {
                                matches.push(Pair {
                                    display: format!(
                                        "{flag_part}  (model info, then specify model)"
                                    ),
                                    replacement: format!("{flag_part} "),
                                });
                                matches.sort_by(|a, b| a.display.cmp(&b.display));
                                return Ok((start, matches));
                            }
                            // If "-i " or "--info " is fully typed, let the next completion step handle model names
                            if prefix_after_flag == " " {
                                return Ok((start_of_model, Vec::new()));
                            }
                            matches.sort_by(|a, b| a.display.cmp(&b.display));
                            return Ok((start, matches));
                        }
                        // Suggest -u / --update and -i / --info flags
                        if arg_prefix.starts_with('-') {
                            for flag in &["-i", "--info", "-u", "--update"] {
                                if flag.starts_with(arg_prefix) {
                                    matches.push(Pair {
                                        display: flag.to_string(),
                                        replacement: format!("{flag} "),
                                    });
                                }
                            }
                            matches.sort_by(|a, b| a.display.cmp(&b.display));
                            return Ok((start, matches));
                        }
                        // Suggest ALL provider:model pairs, sorted
                        let mut providers: Vec<&String> = models_map.keys().collect();
                        providers.sort();
                        for p in providers {
                            if let Some(models) = models_map.get(p) {
                                let mut sorted_models = models.clone();
                                sorted_models.sort();
                                for m in sorted_models {
                                    let entry = format!("{p}:{m}");
                                    if entry.starts_with(arg_prefix) {
                                        matches.push(Pair {
                                            display: entry.clone(),
                                            replacement: entry,
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
                    "/verifier" | "/v" => {
                        let parts: Vec<&str> = arg_prefix.split_whitespace().collect();
                        if parts.is_empty() || (parts.len() == 1 && !arg_prefix.ends_with(' ')) {
                            // Completing subcommand: add, delete, list
                            let prefix = parts.first().copied().unwrap_or("");
                            let subs = ["add", "delete", "list"];
                            let mut matches = Vec::new();
                            for sub in &subs {
                                if sub.starts_with(prefix) {
                                    matches.push(Pair {
                                        display: sub.to_string(),
                                        replacement: format!("{sub} "),
                                    });
                                }
                            }
                            matches.sort_by(|a, b| a.display.cmp(&b.display));
                            return Ok((start, matches));
                        }
                        if parts.len() == 1 && arg_prefix.ends_with(' ') {
                            // "/verifier " → complete subcommand
                            let subs = ["add", "delete", "list"];
                            let mut matches = Vec::new();
                            for sub in &subs {
                                matches.push(Pair {
                                    display: sub.to_string(),
                                    replacement: format!("{sub} "),
                                });
                            }
                            matches.sort_by(|a, b| a.display.cmp(&b.display));
                            return Ok((pos, matches));
                        }
                        if (parts[0] == "add"
                            || parts[0] == "delete"
                            || parts[0] == "del"
                            || parts[0] == "remove"
                            || parts[0] == "rm")
                            && parts.len() >= 2
                        {
                            // Completing provider:model after "add" or "delete"
                            let target_prefix = if parts.len() == 2 && !arg_prefix.ends_with(' ') {
                                parts[1]
                            } else if parts.len() == 2 && arg_prefix.ends_with(' ') {
                                // After a space, complete from empty
                                ""
                            } else {
                                return Ok((0, Vec::new()));
                            };
                            let start_of_target = pos - target_prefix.len();
                            let models_map = self.ctx.config_manager.get_cached_models_sync();
                            let mut matches = Vec::new();
                            // Suggest provider:model pairs
                            let mut providers: Vec<&String> = models_map.keys().collect();
                            providers.sort();
                            for p in providers {
                                if let Some(models) = models_map.get(p) {
                                    let mut sorted_models = models.clone();
                                    sorted_models.sort();
                                    for m in sorted_models {
                                        let entry = format!("{p}:{m}");
                                        if entry.starts_with(target_prefix) {
                                            matches.push(Pair {
                                                display: entry.clone(),
                                                replacement: entry,
                                            });
                                        }
                                    }
                                }
                            }
                            matches.sort_by(|a, b| a.display.cmp(&b.display));
                            matches.dedup_by(|a, b| a.display == b.display);
                            return Ok((start_of_target, matches));
                        }
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
            if self.commands.contains(&cmd.to_string()) {
                let mut highlighted = line.to_string();
                // Use ANSI codes directly to avoid conflicts with rustyline's own handling
                //  is bold cyan,  is reset
                let colored_cmd = cmd.to_string();
                highlighted = highlighted.replacen(cmd, &colored_cmd, 1);
                return std::borrow::Cow::Owned(highlighted);
            }
        }
        // Basic highlighting for markdown-style code blocks in input
        if line.contains("```") {
            let highlighted = line.to_string();
            // No-op: kept as is
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
