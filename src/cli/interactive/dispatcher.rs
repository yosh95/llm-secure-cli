use crate::cli::commands::credits;
use crate::cli::ui;
use crate::core::session::ActiveSession;
use crate::llm::models::{DataSource, Message, MessagePart, Role};
use chrono;
use colored::Colorize;
use serde::{Deserialize, Serialize};
use serde_json;

#[derive(Serialize, Deserialize)]
struct ConversationDump {
    messages: Vec<Message>,
}

pub enum CommandResult {
    Handled,
    NotACommand,
    Exit,
    Input(String),
}

pub async fn handle_command(session: &mut ActiveSession, input: &str) -> CommandResult {
    if !input.starts_with('/') {
        return CommandResult::NotACommand;
    }

    let parts: Vec<&str> = input[1..].splitn(2, ' ').collect();
    let cmd = parts[0].to_lowercase();
    let args = if parts.len() > 1 { parts[1].trim() } else { "" };

    match cmd.as_str() {
        "h" | "help" => {
            print_help();
            CommandResult::Handled
        }
        "q" | "quit" => CommandResult::Exit,

        "edit" | "e" => match ui::open_external_editor(args) {
            Ok(content) => {
                if content.trim().is_empty() {
                    ui::report_warning("Empty input from editor, skipping.");
                    CommandResult::Handled
                } else {
                    CommandResult::Input(content)
                }
            }
            Err(e) => {
                ui::report_error(&format!("Failed to open editor: {e}"));
                CommandResult::Handled
            }
        },
        "clear" | "c" => {
            session
                .get_client_mut()
                .get_state_mut()
                .conversation
                .clear();
            ui::report_success("Conversation history cleared.");
            CommandResult::Handled
        }
        "info" | "i" => {
            handle_info(session);
            CommandResult::Handled
        }
        "raw" => {
            handle_raw(session);
            CommandResult::Handled
        }
        "edit_history" | "eh" => {
            handle_edit_history(session);
            CommandResult::Handled
        }
        "session" => {
            handle_session_cmd(session, args);
            CommandResult::Handled
        }
        "attach" => {
            handle_attach(session, args).await;
            CommandResult::Handled
        }
        "tools" => {
            handle_tools(session, args).await;
            CommandResult::Handled
        }
        "model" | "m" => {
            handle_model_cmd(session, args).await;
            CommandResult::Handled
        }
        "provider" | "p" => {
            handle_provider_cmd(session, args).await;
            CommandResult::Handled
        }
        "summarize" | "s" => {
            handle_summarize(session).await;
            CommandResult::Handled
        }
        "alias" => {
            handle_alias_cmd(session, args).await;
            CommandResult::Handled
        }
        "verify" | "verifier" => {
            handle_verify_cmd(session, args);
            CommandResult::Handled
        }
        "vcommittee" | "vcom" => {
            handle_vcommittee_cmd(session, args).await;
            CommandResult::Handled
        }
        "t" | "template" => {
            let templates = session.ctx.config_manager.load_templates();
            if args.is_empty() {
                if templates.is_empty() {
                    ui::report_info(
                        "No templates found. Place .txt or .md files in ~/.llm_secure_cli/templates/",
                    );
                } else {
                    ui::print_rule(Some("Available Templates"), Some("cyan"));
                    let mut names: Vec<_> = templates.keys().collect();
                    names.sort();
                    for name in names {
                        let preview: String = templates[name]
                            .lines()
                            .find(|l| !l.trim().is_empty())
                            .map_or_else(
                                || "(empty)".to_string(),
                                |l| {
                                    let trimmed = l.trim();
                                    if trimmed.chars().count() > 60 {
                                        format!(
                                            "{}...",
                                            trimmed.chars().take(60).collect::<String>()
                                        )
                                    } else {
                                        trimmed.to_string()
                                    }
                                },
                            );
                        println!("  {: <25} {}", name.bold().cyan(), preview.dimmed());
                    }
                    ui::print_rule(None, Some("cyan"));
                    println!(
                        "{}",
                        "Usage: /t <name>  — insert template into prompt".dimmed()
                    );
                }
                CommandResult::Handled
            } else if let Some(content) = templates.get(args) {
                CommandResult::Input(content.clone())
            } else {
                ui::report_error(&format!(
                    "Template '{args}' not found. Use /t to list available templates."
                ));
                CommandResult::Handled
            }
        }
        "view" => {
            handle_view_cmd(session, args).await;
            CommandResult::Handled
        }
        "tool_output" | "to" => {
            handle_tool_output_cmd(session, args);
            CommandResult::Handled
        }
        "credits" => {
            credits::run_credits_interactive(session).await;
            CommandResult::Handled
        }
        _ => {
            let full_cmd = format!("/{cmd}");
            if !crate::cli::interactive::commands::is_valid_command(&full_cmd) {
                ui::report_error(&format!(
                    "Unknown command: /{cmd}. Type /help for available commands."
                ));
            }
            CommandResult::Handled
        }
    }
}

pub fn handle_session_cmd(session: &mut ActiveSession, args: &str) {
    let args_trimmed = args.trim();

    // No subcommand: list sessions
    if args_trimmed.is_empty() {
        match crate::utils::session_store::list_sessions() {
            Ok(sessions) => {
                if sessions.is_empty() {
                    ui::report_info(
                        "No saved sessions found. Sessions are auto-saved after each turn.",
                    );
                } else {
                    ui::print_rule(Some("Saved Sessions"), Some("cyan"));
                    for s in &sessions {
                        let ts = if s.created_at.is_empty() {
                            "unknown".dimmed().to_string()
                        } else {
                            chrono::DateTime::parse_from_rfc3339(&s.created_at)
                                .map_or_else(
                                    |_| s.created_at.clone(),
                                    |dt| {
                                        dt.with_timezone(&chrono::Local)
                                            .format("%Y-%m-%d %H:%M")
                                            .to_string()
                                    },
                                )
                                .dimmed()
                                .to_string()
                        };
                        let first = s.first_user_prompt.as_deref().unwrap_or("(no user prompt)");
                        println!(
                            "  {}  {: <36} {}",
                            ts,
                            s.filename.bold().cyan(),
                            first.dimmed()
                        );
                    }
                    ui::print_rule(None, Some("cyan"));
                    println!(
                        "{}",
                        "Usage: /session load|delete <id>  or  /session clear".dimmed()
                    );
                }
            }
            Err(e) => ui::report_error(&format!("Failed to list sessions: {e}")),
        }
        return;
    }

    // Parse subcommand
    let parts: Vec<&str> = args_trimmed.splitn(2, ' ').collect();
    let subcmd = parts[0].to_lowercase();
    let subargs = if parts.len() > 1 { parts[1].trim() } else { "" };

    match subcmd.as_str() {
        "load" => {
            if subargs.is_empty() {
                ui::report_error("Usage: /session load <session_id>");
                return;
            }
            match crate::utils::session_store::load_session(subargs) {
                Ok(conversation) => {
                    let client = session.get_client_mut();
                    client.get_state_mut().conversation = conversation;
                    ui::report_success(&format!("Session loaded from {subargs}"));
                }
                Err(e) => ui::report_error(&format!("Failed to load session: {e}")),
            }
        }
        "delete" => {
            if subargs.is_empty() {
                ui::report_error("Usage: /session delete <session_id>");
                return;
            }
            match crate::utils::session_store::delete_session(subargs) {
                Ok(true) => {
                    ui::report_success(&format!("Session '{subargs}' deleted."));
                }
                Ok(false) => {
                    ui::report_error(&format!("Session '{subargs}' not found."));
                }
                Err(e) => ui::report_error(&format!("Failed to delete session: {e}")),
            }
        }
        "clear" => match crate::utils::session_store::clear_sessions() {
            Ok(0) => {
                ui::report_info("No sessions to clear.");
            }
            Ok(n) => {
                ui::report_success(&format!("Cleared {n} session(s)."));
            }
            Err(e) => ui::report_error(&format!("Failed to clear sessions: {e}")),
        },
        _ => {
            ui::report_error(&format!(
                "Unknown subcommand: '{subcmd}'. Use: load, delete, clear"
            ));
        }
    }
}

pub async fn handle_alias_cmd(session: &mut ActiveSession, args: &str) {
    let args_trimmed = args.trim();

    // `/alias` (no args): list all aliases
    if args_trimmed.is_empty() {
        match session.ctx.config_manager.get_state() {
            Ok(state) => {
                if state.model_aliases.is_empty() {
                    ui::report_info("No aliases configured.");
                } else {
                    ui::print_rule(Some("Configured Model Aliases"), Some("cyan"));
                    let mut aliases: Vec<_> = state.model_aliases.iter().collect();
                    aliases.sort_by_key(|(k, _)| *k);
                    for (name, alias) in aliases {
                        println!("  {: <15} -> {}", name.bold().cyan(), alias.target);
                    }
                    ui::print_rule(None, Some("cyan"));
                }
            }
            Err(e) => ui::report_error(&format!("Failed to load aliases: {e}")),
        }
        return;
    }

    let parts: Vec<&str> = args_trimmed.split_whitespace().collect();

    // `/alias -d <name>` or `/alias --delete <name>`: remove an alias
    if parts[0] == "-d" || parts[0] == "--delete" {
        if parts.len() != 2 {
            ui::report_error("Usage: /alias -d <name>");
            return;
        }
        let alias_name = parts[1];
        match session.ctx.config_manager.remove_alias(alias_name) {
            Ok(true) => ui::report_success(&format!("Alias '{alias_name}' removed.")),
            Ok(false) => ui::report_info(&format!("Alias '{alias_name}' does not exist.")),
            Err(e) => ui::report_error(&format!("Failed to remove alias: {e}")),
        }
        return;
    }

    // `/alias <name>`: show a specific alias
    if parts.len() == 1 {
        let alias_name = parts[0];
        match session.ctx.config_manager.get_state() {
            Ok(state) => {
                if let Some(alias) = state.model_aliases.get(alias_name) {
                    println!("  {: <15} -> {}", alias_name.bold().cyan(), alias.target);
                } else {
                    ui::report_info(&format!("Alias '{alias_name}' does not exist."));
                }
            }
            Err(e) => ui::report_error(&format!("Failed to load aliases: {e}")),
        }
        return;
    }

    // `/alias <name> <target>`: create or update an alias
    if parts.len() != 2 {
        ui::report_error(
            "Usage: /alias <name> <target>   — create/update\n       /alias -d <name>       — delete\n       /alias <name>          — show one\n       /alias                 — list all",
        );
        return;
    }

    let alias_name = parts[0];
    let target = parts[1];

    match session.ctx.config_manager.set_alias(alias_name, target) {
        Ok(()) => ui::report_success(&format!("Alias '{alias_name}' set to '{target}'")),
        Err(e) => ui::report_error(&format!("Failed to set alias: {e}")),
    }
}

pub fn handle_info(session: &ActiveSession) {
    let state = session.get_client().get_state();
    let config = match session.ctx.config_manager.get_config() {
        Ok(c) => c,
        Err(e) => {
            ui::report_error(&format!("Failed to load config: {e}"));
            return;
        }
    };

    ui::print_rule(Some("Session Info"), Some("cyan"));
    ui::print_key_value("Session ID", &session.trace_id);
    ui::print_key_value("Model", &state.model);
    ui::print_key_value("Provider", &state.provider);

    // Validator Info
    let (members, _) = session.ctx.config_manager.get_verifier_committee();
    let v_enabled = session.ctx.config_manager.get_verifier_enabled();

    if members.is_empty() {
        let status = if v_enabled {
            "NOT SET (Falling back to manual approval)"
                .red()
                .to_string()
        } else {
            "Not Set".to_string()
        };
        ui::print_key_value("Verifier", &status);
    } else {
        let count = members.len();
        let label = if count == 1 {
            "Verifier"
        } else {
            "Verifier Committee"
        };
        let (p, m) = &members[0];
        if count == 1 {
            ui::print_key_value(label, &format!("{p}:{m}"));
        } else {
            ui::print_key_value(label, &format!("{count} members (any-flag)"));
            ui::print_key_value("  Primary", &format!("{p}:{m}"));
            for (i, (p, m)) in members.iter().enumerate().skip(1) {
                ui::print_key_value(&format!("  Member {i}"), &format!("{p}:{m}"));
            }
        }
    }
    let v_status = if v_enabled {
        "ENABLED".green().to_string()
    } else {
        "DISABLED".yellow().to_string()
    };
    ui::print_key_value("Verifier Status", &v_status);

    // Tool Output display status
    let show_output = session.ctx.config_manager.get_show_tool_result();
    ui::print_key_value(
        "Tool Output",
        if show_output { "Visible" } else { "Hidden" },
    );

    // Security & Integrity
    let integrity_status = match crate::security::integrity::IntegrityVerifier::new().verify() {
        Ok(true) => "Verified (PQC-Signed)".green().to_string(),
        Ok(false) => "TAMPERED/Mismatch".red().to_string(),
        Err(_) => "Not Established (Run /identity manifest)"
            .yellow()
            .to_string(),
    };
    ui::print_key_value("System Integrity", &integrity_status);
    ui::print_key_value("Security Level", config.security.security_level.as_str());

    ui::print_rule(Some("Statistics"), Some("cyan"));
    ui::print_key_value(
        "Usage (Session)",
        &format!(
            "{} prompt / {} completion / {} total tokens",
            crate::utils::format_number(session.total_usage.prompt_tokens),
            crate::utils::format_number(session.total_usage.completion_tokens),
            crate::utils::format_number(session.total_usage.total_tokens)
        ),
    );

    ui::print_rule(Some("Status"), Some("cyan"));
    ui::print_key_value(
        "History",
        &format!(
            "{} messages",
            crate::utils::format_number(state.conversation.len())
        ),
    );
    ui::print_key_value(
        "Tools",
        if state.tools_enabled {
            "Enabled"
        } else {
            "Disabled"
        },
    );

    ui::print_rule(None, Some("cyan"));
}

pub fn handle_raw(session: &ActiveSession) {
    let state = session.get_client().get_state();
    for msg in &state.conversation {
        let role = match msg.role {
            Role::Assistant | Role::Model => &state.model,
            Role::User => "USER",
            Role::System => "SYSTEM",
            Role::Tool => "TOOL",
        };
        println!("[{}]\n{}\n", role, msg.get_text(true));
    }
}

pub fn handle_edit_history(session: &mut ActiveSession) {
    let state = session.get_client().get_state();

    let mut conversation = state.conversation.clone();
    let mut blobs = std::collections::HashMap::new();
    mask_base64_in_conversation(&mut conversation, &mut blobs);

    let initial_content = match toml::to_string(&ConversationDump {
        messages: conversation,
    }) {
        Ok(toml_str) => toml_str,
        Err(e) => {
            ui::report_error(&format!("Failed to serialize conversation: {e}"));
            return;
        }
    };

    match ui::open_external_editor(&initial_content) {
        Ok(edited_toml) => {
            if edited_toml.trim() == initial_content.trim() {
                ui::report_info("No changes made to conversation history.");
                return;
            }

            match toml::from_str::<ConversationDump>(&edited_toml) {
                Ok(mut dump) => {
                    unmask_base64_in_conversation(&mut dump.messages, &blobs);
                    session.get_client_mut().get_state_mut().conversation = dump.messages;
                    ui::report_success("Conversation history updated.");
                }
                Err(e) => ui::report_error(&format!("Failed to parse edited TOML: {e}")),
            }
        }
        Err(e) => ui::report_error(&format!("Failed to open editor: {e}")),
    }
}

fn mask_base64_in_conversation(
    conversation: &mut [Message],
    blobs: &mut std::collections::HashMap<String, serde_json::Value>,
) {
    for msg in conversation {
        for part in &mut msg.parts {
            if let MessagePart::Part(cp) = part
                && let Some(inline_data) = &mut cp.inline_data
                && let Some(data) = inline_data.get_mut("data")
                && let Some(s) = data.as_str()
                && s.len() > 100
            {
                let id = format!("blob_{}", blobs.len());
                blobs.insert(id.clone(), data.clone());
                *data = serde_json::Value::String(format!("<< {id} >>"));
            }
        }
    }
}

fn unmask_base64_in_conversation(
    conversation: &mut [Message],
    blobs: &std::collections::HashMap<String, serde_json::Value>,
) {
    for msg in conversation {
        for part in &mut msg.parts {
            if let MessagePart::Part(cp) = part
                && let Some(inline_data) = &mut cp.inline_data
                && let Some(data) = inline_data.get_mut("data")
                && let Some(s) = data.as_str()
                && s.starts_with("<< blob_")
                && s.ends_with(" >>")
            {
                let id = s.trim_matches(|c| c == '<' || c == '>' || c == ' ');
                if let Some(original_data) = blobs.get(id) {
                    *data = original_data.clone();
                }
            }
        }
    }
}

pub async fn handle_attach(session: &mut ActiveSession, source: &str) {
    if source.is_empty() {
        ui::report_error("Usage: /attach <path_or_url>");
        return;
    }

    let pdf_as_base64 = session.get_client().should_send_pdf_as_base64();
    let data = crate::utils::media::process_single_source(source, pdf_as_base64).await;
    if let Some(d) = data {
        ui::report_success(&format!("Attached {}: {}", d.content_type, source));
        session.pending_data.push(d);
        ui::report_info(
            "File queued. Type your question about it before sending (e.g. \"Summarize this PDF\").",
        );
    } else {
        ui::report_error(&format!("Failed to attach: {source}"));
    }
}

pub async fn handle_tools(session: &mut ActiveSession, args: &str) {
    let state = session.get_client_mut().get_state_mut();
    match args.to_lowercase().as_str() {
        "on" => {
            state.tools_enabled = true;
            ui::report_success("Tools enabled.");
        }
        "off" => {
            state.tools_enabled = false;
            ui::report_success("Tools disabled.");
        }
        "" => {
            let status = if state.tools_enabled {
                "ENABLED"
            } else {
                "DISABLED"
            };
            println!("Tools Status: {status}");
            let registry = session.ctx.tool_registry.read().await;
            println!("Available Tools:");
            for name in registry.tools.keys() {
                println!(" - {name}");
            }
        }
        _ => ui::report_error("Usage: /tools [on|off]"),
    }
}

pub fn handle_verify_cmd(session: &mut ActiveSession, args: &str) {
    let current = session.ctx.config_manager.get_verifier_enabled();

    match args.to_lowercase().as_str() {
        "on" => {
            if let Err(e) = session.ctx.config_manager.set_verifier_enabled(true) {
                ui::report_error(&format!("Failed to update verifier state: {e}"));
            } else {
                ui::report_success("Verifier enabled. (Persisted to state.toml)");
            }
        }
        "off" => {
            if let Err(e) = session.ctx.config_manager.set_verifier_enabled(false) {
                ui::report_error(&format!("Failed to update verifier state: {e}"));
            } else {
                ui::report_success("Verifier disabled. (Persisted to state.toml)");
            }
        }
        "" => {
            let status = if current {
                "ENABLED".green()
            } else {
                "DISABLED".yellow()
            };
            println!("Verifier Status: {status}");

            let (members, _) = session.ctx.config_manager.get_verifier_committee();
            if current {
                if members.is_empty() {
                    ui::report_warning(
                        "Verifier not configured. Use /vcommittee set <provider:model> to configure.",
                    );
                } else if members.len() == 1 {
                    let (p, m) = &members[0];
                    println!("  Verifier: {p}:{m}");
                } else {
                    println!(
                        "  Verifier Committee ({} members, any-flag):",
                        members.len()
                    );
                    for (i, (p, m)) in members.iter().enumerate() {
                        println!("    {}. {}:{}", i + 1, p, m);
                    }
                }
            }
        }
        _ => ui::report_error("Usage: /verify [on|off]"),
    }
}

pub async fn handle_model_cmd(session: &mut ActiveSession, args: &str) {
    let (provider, current_model, stdout, raw) = {
        let state = session.get_client().get_state();
        (
            state.provider.clone(),
            state.model.clone(),
            state.stdout,
            !state.render_markdown,
        )
    };

    let args_trimmed = args.trim();

    // `/model -u` or `/model --update`: refresh the models cache
    if args_trimmed == "-u" || args_trimmed == "--update" {
        ui::report_info("Updating models cache...");
        session.ctx.config_manager.update_models_cache().await;
        ui::report_success("Models cache updated.");
        // fall through to display
    }

    if args_trimmed.is_empty() || args_trimmed == "-u" || args_trimmed == "--update" {
        ui::print_rule(
            Some(&format!("Available Models for {provider}")),
            Some("cyan"),
        );
        let models_map = session.ctx.config_manager.get_cached_models().await;
        if let Some(mut models) = models_map.get(&provider).cloned() {
            models.sort();
            for model in models {
                if model == current_model {
                    println!("  {} {}", "●".cyan(), model.bold().cyan());
                } else {
                    println!("    {model}");
                }
            }
        } else {
            println!("  No models cached for {provider}. Use /model -u to fetch models now.");
        }

        let state = match session.ctx.config_manager.get_state() {
            Ok(s) => s,
            Err(_) => return,
        };
        let mut filtered_aliases: Vec<_> = state
            .model_aliases
            .iter()
            .filter(|(_, v)| v.target.starts_with(&provider))
            .collect();

        if !filtered_aliases.is_empty() {
            println!("\nConfigured Aliases:");
            filtered_aliases.sort_by_key(|(k, _)| *k);
            for (name, alias) in filtered_aliases {
                println!("  {} -> {}", name, alias.target);
            }
        }
        return;
    }

    // Validate: model must exist in cache or be a known alias
    let target_model = args_trimmed;
    let models_map = session.ctx.config_manager.get_cached_models().await;
    let cached_models: Vec<String> = models_map.get(&provider).cloned().unwrap_or_default();
    let state = match session.ctx.config_manager.get_state() {
        Ok(s) => s,
        Err(_) => return,
    };
    let is_alias = state.model_aliases.contains_key(target_model);
    let is_cached = cached_models.iter().any(|m| m == target_model);

    if !is_cached && !is_alias {
        ui::report_error(&format!(
            "Unknown model: '{target_model}'. Use /model to list available models for provider '{provider}'."
        ));
        return;
    }

    match crate::core::initializer::switch_model(session, target_model, stdout, !raw).await {
        Ok(()) => {
            let state = session.get_client().get_state();
            ui::report_success(&format!(
                "Model switched to: {} ({})",
                state.model, state.provider
            ));
        }
        Err(e) => ui::report_error(&format!("Failed to switch model to: {e}")),
    }
}

pub async fn handle_provider_cmd(session: &mut ActiveSession, args: &str) {
    let current_provider = session.get_client().get_state().provider.clone();

    if args.is_empty() {
        ui::print_rule(Some("Available Providers"), Some("cyan"));
        let providers = session.ctx.client_registry.lock().await.list_providers();
        for p in providers {
            if p == current_provider {
                println!("  {} {}", "●".cyan(), p.bold().cyan());
            } else {
                println!("    {p}");
            }
        }
        return;
    }

    let target_provider = args;
    match crate::core::initializer::switch_provider(session, target_provider).await {
        Ok(()) => {
            let state = session.get_client().get_state();
            ui::report_success(&format!(
                "Provider switched to: {} (Model: {})",
                state.provider, state.model
            ));
        }
        Err(e) => ui::report_error(&format!("Failed to switch provider: {e}")),
    }
}

pub async fn handle_vcommittee_cmd(session: &mut ActiveSession, args: &str) {
    let parts: Vec<&str> = args.split_whitespace().collect();

    match parts.first().copied().unwrap_or("") {
        "set" if parts.len() >= 2 => {
            let provider_model = parts[1..].join(" ");
            if !provider_model.contains(':') {
                ui::report_error(
                    "Usage: /vcommittee set <provider:model> (e.g. ollama:gemma4:e2b)",
                );
                return;
            }
            if let Err(e) = session
                .ctx
                .config_manager
                .set_primary_verifier(&provider_model)
            {
                ui::report_error(&format!("Failed to set verifier: {e}"));
            } else {
                ui::report_success(&format!("Verifier set to: {provider_model}"));
            }
        }
        "add" if parts.len() >= 2 => {
            let provider_model = parts[1..].join(" ");
            if !provider_model.contains(':') {
                ui::report_error(
                    "Usage: /vcommittee add <provider:model> (e.g. openai:gpt-4o-mini)",
                );
                return;
            }
            if let Err(e) = session
                .ctx
                .config_manager
                .add_verifier_committee_member(&provider_model)
            {
                ui::report_error(&format!("Failed to add committee member: {e}"));
            } else {
                ui::report_success(&format!("Added committee member: {provider_model}"));
            }
        }
        "remove" | "rm" if parts.len() >= 2 => {
            let provider_model = parts[1..].join(" ");
            match session
                .ctx
                .config_manager
                .remove_verifier_committee_member(&provider_model)
            {
                Ok(true) => {
                    ui::report_success(&format!("Removed committee member: {provider_model}"));
                }
                Ok(false) => {
                    ui::report_warning(&format!("Committee member not found: {provider_model}"));
                }
                Err(e) => ui::report_error(&format!("Failed to remove committee member: {e}")),
            }
        }
        "list" | "ls" | "" => {
            let (members, _enabled) = session.ctx.config_manager.get_verifier_committee();
            let verifier_enabled = session.ctx.config_manager.get_verifier_enabled();

            ui::print_rule(Some("Verifier"), Some("cyan"));
            let status = if verifier_enabled {
                "ENABLED".green()
            } else {
                "DISABLED".yellow()
            };
            println!("  Status: {status}");

            if members.is_empty() {
                println!("  No committee members configured.");
                if verifier_enabled {
                    println!("  (Falling back to manual approval for all tool calls)");
                }
            } else {
                println!(
                    "  Committee (any-flag policy, {} member(s)):",
                    members.len()
                );
                for (provider, model) in &members {
                    let prefix = "  ── ";
                    println!("{prefix}{provider}:{model}");
                }
            }
            println!();
            println!("  Commands:");
            println!("    /vcommittee set <provider:model>       Set primary (replaces all)");
            println!("    /vcommittee add <provider:model>       Add committee member");
            println!("    /vcommittee remove <provider:model>    Remove committee member");
        }
        _ => ui::report_error("Usage: /vcommittee [set|add|remove|list] [<provider:model>]"),
    }
}

pub async fn handle_summarize(session: &mut ActiveSession) {
    let history_len = session.get_client().get_state().conversation.len();
    if history_len == 0 {
        ui::report_warning("Conversation is empty, nothing to summarize.");
        return;
    }

    ui::report_info("Summarizing conversation and clearing history...");

    let summary_prompt = "Please provide a concise summary of the conversation so far. This summary will be used as context for future interactions. IMPORTANT: The summary must be written in the same language as the conversation (e.g., if the user is speaking Japanese, summarize in Japanese).";

    // Prepare data source for summarization
    let data = vec![DataSource {
        content: serde_json::Value::String(summary_prompt.to_string()),
        content_type: "text/plain".to_string(),
        is_file_or_url: false,
        metadata: std::collections::HashMap::new(),
    }];

    // We use the empty tool_schemas as we just want a summary
    match session.get_client_mut().send(data, Vec::new()).await {
        Ok(response) => {
            let summary_text = response.content.clone().unwrap_or_default();

            // Reconstruct history with summary
            let mut new_conversation = Vec::new();

            // Add the summary as a historical context rather than a system message
            // to avoid clashing with the dynamic system prompt (which includes the date).
            new_conversation.push(Message {
                role: Role::User,
                parts: vec![MessagePart::Text(format!(
                    "Summary of our previous conversation for context:\n{summary_text}"
                ))],
            });

            new_conversation.push(Message {
                role: Role::Assistant,
                parts: vec![MessagePart::Text(
                    "I have acknowledged the summary and will use it as context for our continued conversation."
                        .to_string(),
                )],
            });

            session.get_client_mut().get_state_mut().conversation = new_conversation;

            ui::report_success("Conversation summarized and history cleared.");
            println!("\n{}\n", "--- Summary ---".cyan());
            println!("{summary_text}");
            println!("{}\n", "---------------".cyan());
        }
        Err(e) => ui::report_error(&format!("Failed to summarize: {e}")),
    }
}

pub async fn handle_view_cmd(session: &mut ActiveSession, args: &str) {
    let config = match session.ctx.config_manager.get_config() {
        Ok(c) => c,
        Err(e) => {
            ui::report_error(&format!("Failed to load config: {e}"));
            return;
        }
    };

    let save_dir = std::path::Path::new(&config.general.image_save_path);

    if args.is_empty() {
        // No argument: find the most recently saved media file
        match crate::utils::media::find_latest_media(save_dir) {
            Some(latest) => match crate::utils::media::open_file_with_default_app(&latest) {
                Ok(()) => ui::report_success(&format!("Opened: {}", latest.display())),
                Err(e) => ui::report_error(&e.to_string()),
            },
            None => {
                ui::report_error(&format!(
                    "No saved media found in {}. Generate an image first.",
                    save_dir.display()
                ));
            }
        }
    } else {
        // Argument: treat as a file path
        let path = std::path::Path::new(args);
        let path = if path.is_relative() {
            // Try relative to CWD, then relative to the save directory
            let cwd_path = std::env::current_dir().unwrap_or_default().join(args);
            if cwd_path.exists() {
                cwd_path
            } else {
                save_dir.join(args)
            }
        } else {
            path.to_path_buf()
        };

        // Expand ~ if present
        let path = if path.starts_with("~") {
            if let Some(home) = dirs::home_dir() {
                if let Ok(stripped) = path.strip_prefix("~") {
                    home.join(stripped)
                } else {
                    path
                }
            } else {
                path
            }
        } else {
            path
        };

        if !path.exists() {
            ui::report_error(&format!("File not found: {}", path.display()));
            return;
        }

        match crate::utils::media::open_file_with_default_app(&path) {
            Ok(()) => ui::report_success(&format!("Opened: {}", path.display())),
            Err(e) => ui::report_error(&e.to_string()),
        }
    }
}

pub fn handle_tool_output_cmd(session: &mut ActiveSession, args: &str) {
    match args.to_lowercase().as_str() {
        "on" | "show" => {
            if let Err(e) = session.ctx.config_manager.set_show_tool_result(true) {
                ui::report_error(&format!("Failed to update tool output setting: {e}"));
            } else {
                ui::report_success(
                    "Tool execution results will now be displayed. (Persisted to state.toml)",
                );
            }
        }
        "off" | "hide" => {
            if let Err(e) = session.ctx.config_manager.set_show_tool_result(false) {
                ui::report_error(&format!("Failed to update tool output setting: {e}"));
            } else {
                ui::report_success(
                    "Tool execution results will now be hidden. (Persisted to state.toml)",
                );
            }
        }
        "" => {
            let current = session.ctx.config_manager.get_show_tool_result();
            let status = if current {
                "VISIBLE".green()
            } else {
                "HIDDEN".yellow()
            };
            println!("Tool Output Status: {status}");
            println!("  When hidden, tool execution results are not shown in the terminal");
            println!("  (they are still sent to the LLM and logged to the audit trail).");
        }
        _ => ui::report_error("Usage: /tool_output [on|off] (or [show|hide])"),
    }
}

fn print_help() {
    ui::print_rule(Some("Interactive Commands"), Some("cyan"));
    println!("  /h, /help          Show this help message");
    println!("  /q, /quit          Exit the session");
    println!("  /i, /info          Show session and security status");
    println!("  /c, /clear         Clear conversation history");
    println!("  /e, /edit          Open external editor for multi-line input");
    println!(
        "  /eh, /edit_history View/edit the conversation history in TOML format (includes full structure)"
    );
    println!("  /session [load|delete <id>|clear]  List, load, delete, or clear saved sessions");
    println!("  /attach <path|url> Attach a file or URL to the next request");
    println!(
        "  /tools [on|off]    Toggle or show status of tool execution
  /to, /tool_output [on|off] Toggle display of tool execution results (default: hidden)"
    );

    println!("  /m, /model [-u] [<name>]  List models (/model -u to refresh cache) or switch");
    println!("  /p, /provider <n>  Switch LLM provider");
    println!(
        "  /vcommittee [set|add|remove|list] [<provider:model>]  Manage verifier (set=replace all, add=add member)"
    );
    println!("  /alias [-d <name>] [<name> <target>]  List/create/delete model aliases");
    println!("  /s, /summarize     Summarize history and clear it");
    println!("  /t, /template [<name>]  List templates or insert one into prompt");
    println!(
        "  /view [<path>]      Open saved image or file with system default app (no arg = latest)"
    );
    println!(
        "  /credits          Show detailed OpenRouter credit info (uses both /credits and /key APIs)"
    );
    println!("  /raw               Show raw conversation history");
    ui::print_rule(None, Some("cyan"));
}
