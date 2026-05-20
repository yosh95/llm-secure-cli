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
                "/summarize",
                "/s",
                "/edit_history",
                "/eh",
                "/alias",
                "/verify",
                "/verifier",
                "/template",
                "/t",
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
                    "/load" => {
                        // Complete session filenames from sessions directory
                        let dir = crate::consts::sessions_dir();
                        let mut matches = Vec::new();
                        if dir.exists()
                            && let Ok(entries) = std::fs::read_dir(&dir)
                        {
                            for entry in entries.flatten() {
                                let path = entry.path();
                                if path.extension().and_then(|s| s.to_str()) == Some("json")
                                    && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                                    && stem.starts_with(arg_prefix)
                                {
                                    // Also get first user prompt for preview
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
                                                        let line = t.lines().next().unwrap_or("");
                                                        if line.chars().count() > 40 {
                                                            format!(
                                                                "{}...",
                                                                line.chars()
                                                                    .take(37)
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
                                        format!("{}  ({})", stem, preview)
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
                        return Ok((start, matches));
                    }
                    "/save" | "/attach" | "/edit" | "/e" => {
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
                    "/model" | "/m" | "/models" => {
                        let models_map = self.ctx.config_manager.get_cached_models_sync();
                        let mut matches = Vec::new();
                        let current_p = self
                            .current_provider
                            .lock()
                            .unwrap_or_else(|e| {
                                tracing::warn!(error = %e, "Provider mutex poisoned during completion");
                                e.into_inner()
                            })
                            .clone();

                        // Suggest -u / --update flag
                        if arg_prefix.starts_with('-') {
                            for flag in &["-u", "--update"] {
                                if flag.starts_with(arg_prefix) {
                                    matches.push(Pair {
                                        display: flag.to_string(),
                                        replacement: format!("{} ", flag),
                                    });
                                }
                            }
                            matches.sort_by(|a, b| a.display.cmp(&b.display));
                            return Ok((start, matches));
                        }

                        // Add aliases to completions (only for main model switch)
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

                        matches.sort_by(|a, b| a.display.cmp(&b.display));
                        matches.dedup_by(|a, b| a.display == b.display);
                        return Ok((start, matches));
                    }
                    "/vmodel" | "/vm" => {
                        let models_map = self.ctx.config_manager.get_cached_models_sync();
                        let mut matches = Vec::new();

                        // Suggest -u / --update flag
                        if arg_prefix.starts_with('-') {
                            for flag in &["-u", "--update"] {
                                if flag.starts_with(arg_prefix) {
                                    matches.push(Pair {
                                        display: flag.to_string(),
                                        replacement: format!("{} ", flag),
                                    });
                                }
                            }
                            matches.sort_by(|a, b| a.display.cmp(&b.display));
                            return Ok((start, matches));
                        }

                        let (v_p, _) = self.ctx.config_manager.get_dual_llm_settings();

                        if !v_p.is_empty()
                            && let Some(models) = models_map.get(&v_p)
                        {
                            for model in models {
                                if model.starts_with(arg_prefix) {
                                    matches.push(Pair {
                                        display: model.clone(),
                                        replacement: model.clone(),
                                    });
                                }
                            }
                        }

                        matches.sort_by(|a, b| a.display.cmp(&b.display));
                        matches.dedup_by(|a, b| a.display == b.display);
                        return Ok((start, matches));
                    }
                    "/tools" | "/system" | "/verify" | "/verifier" => {
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
                    "/t" | "/template" => {
                        let templates = self.ctx.config_manager.load_templates();
                        let mut matches = Vec::new();
                        for name in templates.keys() {
                            if name.starts_with(arg_prefix) {
                                matches.push(Pair {
                                    display: name.clone(),
                                    replacement: name.clone(),
                                });
                            }
                        }
                        matches.sort_by(|a, b| a.display.cmp(&b.display));
                        return Ok((start, matches));
                    }
                    "/alias" => {
                        let parts: Vec<&str> = arg_prefix.split_whitespace().collect();
                        // `/alias -d <name>` or `/alias --delete <name>`: complete alias names
                        if (parts.len() == 2 && (parts[1] == "-d" || parts[1] == "--delete"))
                            && arg_prefix.ends_with(' ')
                        {
                            let state = self.ctx.config_manager.get_state().unwrap_or_default();
                            let mut matches: Vec<Pair> = state
                                .model_aliases
                                .keys()
                                .map(|k| Pair {
                                    display: k.clone(),
                                    replacement: k.clone(),
                                })
                                .collect();
                            matches.sort_by(|a, b| a.display.cmp(&b.display));
                            return Ok((pos, matches));
                        }
                        if parts.len() == 3 && (parts[1] == "-d" || parts[1] == "--delete") {
                            let target_prefix = parts[2];
                            let start_of_target = pos - target_prefix.len();
                            let state = self.ctx.config_manager.get_state().unwrap_or_default();
                            let mut matches: Vec<Pair> = state
                                .model_aliases
                                .keys()
                                .filter(|k| k.starts_with(target_prefix))
                                .map(|k| Pair {
                                    display: k.clone(),
                                    replacement: k.clone(),
                                })
                                .collect();
                            matches.sort_by(|a, b| a.display.cmp(&b.display));
                            return Ok((start_of_target, matches));
                        }
                        // Suggest -d / --delete flag as first argument
                        if parts.len() == 1 && arg_prefix.ends_with(' ') {
                            let mut matches = vec![
                                Pair {
                                    display: "-d (delete an alias)".to_string(),
                                    replacement: "-d ".to_string(),
                                },
                                Pair {
                                    display: "--delete (delete an alias)".to_string(),
                                    replacement: "--delete ".to_string(),
                                },
                            ];
                            // Also add model targets for normal alias creation
                            let models_map = self.ctx.config_manager.get_cached_models_sync();
                            for (p, models) in models_map {
                                for m in models {
                                    let full = format!("{}:{}", p, m);
                                    matches.push(Pair {
                                        display: full.clone(),
                                        replacement: full,
                                    });
                                }
                            }
                            matches.sort_by(|a, b| a.display.cmp(&b.display));
                            return Ok((pos, matches));
                        }
                        // `/alias <name> <partial>`: complete target
                        if parts.len() == 2 {
                            let target_prefix = parts[1];
                            let start_of_target = pos - target_prefix.len();
                            // If it starts with '-', suggest flags
                            if target_prefix.starts_with('-') {
                                let mut matches = Vec::new();
                                for flag in &["-d", "--delete"] {
                                    if flag.starts_with(target_prefix) {
                                        matches.push(Pair {
                                            display: flag.to_string(),
                                            replacement: format!("{} ", flag),
                                        });
                                    }
                                }
                                matches.sort_by(|a, b| a.display.cmp(&b.display));
                                return Ok((start_of_target, matches));
                            }
                            let models_map = self.ctx.config_manager.get_cached_models_sync();
                            let mut matches = Vec::new();
                            for (p, models) in models_map {
                                for m in models {
                                    let full = format!("{}:{}", p, m);
                                    if full.starts_with(target_prefix) {
                                        matches.push(Pair {
                                            display: full.clone(),
                                            replacement: full,
                                        });
                                    }
                                }
                            }
                            matches.sort_by(|a, b| a.display.cmp(&b.display));
                            return Ok((start_of_target, matches));
                        }
                        // parts.len() == 1 (just `/alias` without trailing space)
                        if parts.len() == 1 && !arg_prefix.ends_with(' ') {
                            // nothing extra needed here, the command is complete
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
