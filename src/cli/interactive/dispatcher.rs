use crate::cli::ui;
use crate::core::session::ActiveSession;
use crate::llm::models::{Message, MessagePart, Role};
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
        "system" => {
            let state = session.get_client_mut().get_state_mut();
            match args.to_lowercase().as_str() {
                "on" => {
                    state.system_prompt_enabled = true;
                    ui::report_success("System prompt enabled.");
                }
                "off" => {
                    state.system_prompt_enabled = false;
                    ui::report_success("System prompt disabled.");
                }
                "" => {
                    let status = if state.system_prompt_enabled {
                        "ON"
                    } else {
                        "OFF"
                    };
                    println!("System Prompt Status: {}", status);
                    if let Some(sp) = state.get_effective_system_prompt() {
                        println!("\nEffective System Prompt:\n{}", sp);
                    }
                }
                _ => ui::report_error("Usage: /system [on|off]"),
            }
            CommandResult::Handled
        }
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
                ui::report_error(&format!("Failed to open editor: {}", e));
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
        "dump" => {
            handle_dump(session);
            CommandResult::Handled
        }
        "edit_history" | "eh" => {
            handle_edit_history(session);
            CommandResult::Handled
        }
        "save" => {
            if args.is_empty() {
                ui::report_error("Usage: /save <path>");
            } else {
                let client = session.get_client();
                match client.save_session(args) {
                    Ok(_) => ui::report_success(&format!("Session saved to {}", args)),
                    Err(e) => ui::report_error(&format!("Failed to save session: {}", e)),
                }
            }
            CommandResult::Handled
        }
        "load" => {
            if args.is_empty() {
                ui::report_error("Usage: /load <path>");
            } else {
                let client = session.get_client_mut();
                match client.load_session(args) {
                    Ok(_) => ui::report_success(&format!("Session loaded from {}", args)),
                    Err(e) => ui::report_error(&format!("Failed to load session: {}", e)),
                }
            }
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
        "model" | "models" | "m" => {
            handle_model_cmd(session, args).await;
            CommandResult::Handled
        }
        "vmodel" | "vm" => {
            handle_vmodel_cmd(session, args).await;
            CommandResult::Handled
        }
        "provider" | "p" => {
            handle_provider_cmd(session, args).await;
            CommandResult::Handled
        }
        "vprovider" | "vp" => {
            handle_vprovider_cmd(session, args).await;
            CommandResult::Handled
        }
        "checkpoint" | "cp" => {
            handle_checkpoint(session).await;
            CommandResult::Handled
        }
        _ => {
            ui::report_error(&format!("Unknown command: /{}", cmd));
            CommandResult::Handled
        }
    }
}

pub fn handle_info(session: &ActiveSession) {
    let state = session.get_client().get_state();
    let config = match session.ctx.config_manager.get_config() {
        Ok(c) => c,
        Err(e) => {
            ui::report_error(&format!("Failed to load config: {}", e));
            return;
        }
    };

    ui::print_rule(Some("Session Info"), Some("cyan"));
    ui::print_key_value("Session ID", &session.trace_id);
    ui::print_key_value("Model", &state.model);
    ui::print_key_value("Provider", &state.provider);

    // Validator Info
    let v_enabled = config.security.dual_llm_verification.unwrap_or(false);
    let v_provider = &config.security.dual_llm_provider;
    let v_model = if config.security.dual_llm_model.is_empty() {
        if v_enabled {
            "NOT SET (Falling back to manual approval)"
                .red()
                .to_string()
        } else {
            "Not Set".to_string()
        }
    } else {
        config.security.dual_llm_model.clone()
    };
    ui::print_key_value("Verifier Model", &v_model);
    ui::print_key_value("Verifier Prov", v_provider);
    let v_status = if v_enabled {
        "ENABLED".green().to_string()
    } else {
        "DISABLED".yellow().to_string()
    };
    ui::print_key_value("Verifier Status", &v_status);

    // Security & Integrity
    let integrity_status = match crate::security::integrity::IntegrityVerifier::new().verify() {
        Ok(true) => "Verified (PQC-Signed)".green().to_string(),
        Ok(false) => "TAMPERED/Mismatch".red().to_string(),
        Err(_) => "Not Established (Run /identity manifest)"
            .yellow()
            .to_string(),
    };
    ui::print_key_value("System Integrity", &integrity_status);
    ui::print_key_value("PQC Algorithm", "ML-DSA-65 (Dilithium)");
    ui::print_key_value("Security Level", &config.security.security_level);

    ui::print_rule(Some("Statistics"), Some("cyan"));
    ui::print_key_value(
        "Usage (Session)",
        &format!(
            "{} prompt / {} completion / {} total tokens",
            session.total_usage.prompt_tokens,
            session.total_usage.completion_tokens,
            session.total_usage.total_tokens
        ),
    );

    ui::print_rule(Some("Status"), Some("cyan"));
    ui::print_key_value("History", &format!("{} messages", state.conversation.len()));
    ui::print_key_value(
        "Tools",
        if state.tools_enabled {
            "Enabled"
        } else {
            "Disabled"
        },
    );
    ui::print_key_value(
        "System Prompt",
        if state.system_prompt_enabled {
            "On"
        } else {
            "Off"
        },
    );
    if state.system_prompt_enabled
        && let Some(sp) = state.get_effective_system_prompt()
    {
        println!("  --------------------------------------------------");
        for line in sp.lines() {
            println!("  {}", line);
        }
        println!("  --------------------------------------------------");
    }
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

pub fn handle_dump(session: &ActiveSession) {
    let state = session.get_client().get_state();

    let mut conversation = state.conversation.clone();
    let mut blobs = std::collections::HashMap::new();
    mask_base64_in_conversation(&mut conversation, &mut blobs);

    let dump = ConversationDump {
        messages: conversation,
    };

    match toml::to_string(&dump) {
        Ok(toml_str) => {
            ui::print_rule(Some("Conversation Dump (TOML)"), Some("magenta"));
            print!("{}", toml_str);
            ui::print_rule(None, Some("magenta"));
        }
        Err(e) => ui::report_error(&format!("Failed to dump conversation: {}", e)),
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
            ui::report_error(&format!("Failed to serialize conversation: {}", e));
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
                Err(e) => ui::report_error(&format!("Failed to parse edited TOML: {}", e)),
            }
        }
        Err(e) => ui::report_error(&format!("Failed to open editor: {}", e)),
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
                *data = serde_json::Value::String(format!("<< {} >>", id));
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
        ui::report_error(&format!("Failed to attach: {}", source));
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
            println!("Tools Status: {}", status);
            let registry = session.ctx.tool_registry.lock().await;
            println!("Available Tools:");
            for name in registry.tools.keys() {
                println!(" - {}", name);
            }
        }
        _ => ui::report_error("Usage: /tools [on|off]"),
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

    if args.is_empty() {
        ui::print_rule(
            Some(&format!("Available Models for {}", provider)),
            Some("cyan"),
        );
        let models_map = session.ctx.config_manager.get_cached_models().await;
        if let Some(mut models) = models_map.get(&provider).cloned() {
            models.sort();
            for model in models {
                if model == current_model {
                    println!("  {} {}", "●".cyan(), model.bold().cyan());
                } else {
                    println!("    {}", model);
                }
            }
        } else {
            println!(
                "  No models cached for {}. Try running the provider to fetch models.",
                provider
            );
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

    let target_model = args;
    match crate::core::initializer::switch_model(session, target_model, stdout, !raw).await {
        Ok(_) => {
            let state = session.get_client().get_state();
            ui::report_success(&format!(
                "Model switched to: {} ({})",
                state.model, state.provider
            ));
        }
        Err(e) => ui::report_error(&format!("Failed to switch model to: {}", e)),
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
                println!("    {}", p);
            }
        }
        return;
    }

    let target_provider = args;
    match crate::core::initializer::switch_provider(session, target_provider).await {
        Ok(_) => {
            let state = session.get_client().get_state();
            ui::report_success(&format!(
                "Provider switched to: {} (Model: {})",
                state.provider, state.model
            ));
        }
        Err(e) => ui::report_error(&format!("Failed to switch provider: {}", e)),
    }
}

pub async fn handle_vmodel_cmd(session: &mut ActiveSession, args: &str) {
    let config = match session.ctx.config_manager.get_config() {
        Ok(c) => c,
        Err(e) => {
            ui::report_error(&format!("Failed to load config: {}", e));
            return;
        }
    };
    let current_provider = &config.security.dual_llm_provider;
    let current_model = &config.security.dual_llm_model;

    if args.is_empty() {
        ui::print_rule(
            Some(&format!(
                "Available Models for Verifier ({})",
                current_provider
            )),
            Some("cyan"),
        );
        let models_map = session.ctx.config_manager.get_cached_models().await;
        if let Some(mut models) = models_map.get(current_provider).cloned() {
            models.sort();
            for model in models {
                if &model == current_model {
                    println!("  {} {}", "●".cyan(), model.bold().cyan());
                } else {
                    println!("    {}", model);
                }
            }
        }
        return;
    }

    let mut new_config = (*config).clone();
    new_config.security.dual_llm_model = args.to_string();
    if let Err(e) = session.ctx.config_manager.set_config(new_config) {
        ui::report_error(&format!("Failed to update config: {}", e));
    } else {
        ui::report_success(&format!("Verifier model set to: {}", args));
    }
}

pub async fn handle_vprovider_cmd(session: &mut ActiveSession, args: &str) {
    let config = match session.ctx.config_manager.get_config() {
        Ok(c) => c,
        Err(e) => {
            ui::report_error(&format!("Failed to load config: {}", e));
            return;
        }
    };
    let current_provider = &config.security.dual_llm_provider;

    if args.is_empty() {
        ui::print_rule(Some("Available Providers for Verifier"), Some("cyan"));
        let providers = session.ctx.client_registry.lock().await.list_providers();
        for p in providers {
            if &p == current_provider {
                println!("  {} {}", "●".cyan(), p.bold().cyan());
            } else {
                println!("    {}", p);
            }
        }
        return;
    }

    let mut new_config = (*config).clone();
    new_config.security.dual_llm_provider = args.to_string();
    new_config.security.dual_llm_model = String::new(); // Reset model when provider changes
    if let Err(e) = session.ctx.config_manager.set_config(new_config) {
        ui::report_error(&format!("Failed to update config: {}", e));
    } else {
        ui::report_success(&format!(
            "Verifier provider set to: {}. Please set a model with /vmodel.",
            args
        ));
    }
}

pub async fn handle_checkpoint(session: &mut ActiveSession) {
    let history_len = session.get_client().get_state().conversation.len();

    if history_len == 0 {
        ui::report_warning("Conversation is empty, nothing to checkpoint.");
        return;
    }

    ui::report_info("Creating session checkpoint (PQC-anchored)...");
    let trace_id = &session.trace_id;
    let entries = session
        .audit_entries
        .iter()
        .filter_map(|e| serde_json::to_value(e).ok())
        .collect::<Vec<_>>();

    match crate::security::merkle_anchor::SessionAnchorManager::create_anchor(
        trace_id,
        Some(entries),
    ) {
        Ok(root) => {
            ui::report_success(&format!(
                "Checkpoint created. Merkle Root: {}",
                root.unwrap_or_default()
            ));
            ui::report_info("Integrity of conversation history is now cryptographically anchored.");
        }
        Err(e) => ui::report_error(&format!("Failed to create checkpoint: {}", e)),
    }
}

fn print_help() {
    ui::print_rule(Some("Interactive Commands"), Some("cyan"));
    println!("  /h, /help          Show this help message");
    println!("  /q, /quit          Exit the session");
    println!("  /i, /info          Show session and security status");
    println!("  /c, /clear         Clear conversation history");
    println!("  /e, /edit          Open external editor for multi-line input");
    println!("  /eh, /edit_history Edit the conversation history in TOML format");
    println!("  /save <path>       Save the current session history to a file");
    println!("  /load <path>       Load session history from a file");
    println!("  /attach <path|url> Attach a file or URL to the next request");
    println!("  /tools [on|off]    Toggle or show status of tool execution");
    println!("  /system [on|off]   Toggle or show system prompt status");
    println!("  /m, /model <name>  Switch LLM model for the current provider");
    println!("  /p, /provider <n>  Switch LLM provider");
    println!("  /vm, /vmodel <n>   Set model for dual-LLM verification");
    println!("  /vp, /vprovider <n> Set provider for dual-LLM verification");
    println!("  /cp, /checkpoint   Manually anchor session integrity");
    println!("  /raw               Show raw conversation history");
    println!("  /dump              Dump conversation history as TOML");
    ui::print_rule(None, Some("cyan"));
}
