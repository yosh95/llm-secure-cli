use crate::cli::ui;
use crate::core::session::ChatSession;
use crate::llm::models::{DataSource, Message, MessagePart, Role};
use colored::Colorize;

pub enum CommandResult {
    Handled,
    NotACommand,
    Exit,
    Input(String),
}

pub async fn handle_command(session: &mut ChatSession, input: &str) -> CommandResult {
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
            let state = match session.get_client_mut() {
                Ok(c) => c.get_state_mut(),
                Err(e) => {
                    ui::report_error(&e.to_string());
                    return CommandResult::Handled;
                }
            };
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
            match session.get_client_mut() {
                Ok(client) => {
                    client.get_state_mut().conversation.clear();
                    ui::report_success("Conversation history cleared.");
                }
                Err(e) => ui::report_error(&e.to_string()),
            }
            CommandResult::Handled
        }
        "info" | "i" => {
            handle_info(session);
            CommandResult::Handled
        }
        "debug" => {
            let state = match session.get_client_mut() {
                Ok(c) => c.get_state_mut(),
                Err(e) => {
                    ui::report_error(&e.to_string());
                    return CommandResult::Handled;
                }
            };
            state.live_debug = !state.live_debug;
            let status = if state.live_debug {
                log::set_max_level(log::LevelFilter::Debug);
                "ON"
            } else {
                log::set_max_level(log::LevelFilter::Warn);
                "OFF"
            };
            ui::report_success(&format!("Live debug mode is now {}.", status));
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
        "save" => {
            if args.is_empty() {
                ui::report_error("Usage: /save <path>");
            } else {
                match session.get_client() {
                    Ok(client) => match client.save_session(args) {
                        Ok(_) => ui::report_success(&format!("Session saved to {}", args)),
                        Err(e) => ui::report_error(&format!("Failed to save session: {}", e)),
                    },
                    Err(e) => ui::report_error(&e.to_string()),
                }
            }
            CommandResult::Handled
        }
        "load" => {
            if args.is_empty() {
                ui::report_error("Usage: /load <path>");
            } else {
                match session.get_client_mut() {
                    Ok(client) => match client.load_session(args) {
                        Ok(_) => ui::report_success(&format!("Session loaded from {}", args)),
                        Err(e) => ui::report_error(&format!("Failed to load session: {}", e)),
                    },
                    Err(e) => ui::report_error(&e.to_string()),
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
        "provider" | "p" => {
            handle_provider_cmd(session, args).await;
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

pub fn handle_info(session: &ChatSession) {
    let state = match session.get_client() {
        Ok(client) => client.get_state(),
        Err(e) => {
            ui::report_error(&e.to_string());
            return;
        }
    };
    let config = match session.ctx.config_manager.get_config() {
        Ok(c) => c,
        Err(e) => {
            ui::report_error(&format!("Failed to load config: {}", e));
            return;
        }
    };

    ui::print_rule(Some("Session Info"), Some("cyan"));
    ui::print_key_value("Session ID", &session.trace_id);
    ui::print_key_value("Provider", &state.provider);
    ui::print_key_value("Model", &state.model);

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
    ui::print_key_value(
        "Debug Mode",
        if log::log_enabled!(log::Level::Debug) {
            "On"
        } else {
            "Off"
        },
    );
    ui::print_rule(None, Some("cyan"));
}

pub fn handle_raw(session: &ChatSession) {
    let state = match session.get_client() {
        Ok(client) => client.get_state(),
        Err(e) => {
            ui::report_error(&e.to_string());
            return;
        }
    };
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

pub fn handle_dump(session: &ChatSession) {
    let state = match session.get_client() {
        Ok(client) => client.get_state(),
        Err(e) => {
            ui::report_error(&e.to_string());
            return;
        }
    };
    match serde_json::to_string_pretty(&state.conversation) {
        Ok(json) => {
            ui::print_rule(Some("Conversation Dump"), Some("magenta"));
            println!("{}", json);
            ui::print_rule(None, Some("magenta"));
        }
        Err(e) => ui::report_error(&format!("Failed to dump conversation: {}", e)),
    }
}

pub async fn handle_attach(session: &mut ChatSession, source: &str) {
    if source.is_empty() {
        ui::report_error("Usage: /attach <path_or_url>");
        return;
    }

    let pdf_as_base64 = match session.get_client() {
        Ok(client) => client.should_send_pdf_as_base64(),
        Err(e) => {
            ui::report_error(&e.to_string());
            return;
        }
    };
    let data = crate::utils::media::process_single_source(source, pdf_as_base64).await;
    if let Some(d) = data {
        ui::report_success(&format!("Attached {}: {}", d.content_type, source));
        session.pending_data.push(d);
    } else {
        ui::report_error(&format!("Failed to attach: {}", source));
    }
}

pub async fn handle_tools(session: &mut ChatSession, args: &str) {
    let state = match session.get_client_mut() {
        Ok(client) => client.get_state_mut(),
        Err(e) => {
            ui::report_error(&e.to_string());
            return;
        }
    };
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

pub async fn handle_model_cmd(session: &mut ChatSession, args: &str) {
    let (provider, current_model, stdout, raw) = {
        let state = match session.get_client() {
            Ok(client) => client.get_state(),
            Err(e) => {
                ui::report_error(&e.to_string());
                return;
            }
        };
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
            ui::print_rule(None, Some("cyan"));
        } else {
            println!(
                "  No models cached for {}. Try running the provider to fetch models.",
                provider
            );
        }
    } else {
        let client = {
            let registry = session.ctx.client_registry.lock().await;
            registry.create_client(&provider, args, stdout, raw, &session.ctx.config_manager)
        };

        match client {
            Some(new_client) => {
                session.switch_client(new_client);
                let _ = session.ctx.config_manager.update_state(&provider, args);
                ui::report_success(&format!(
                    "Model switched to: {} ({})",
                    args,
                    match session.get_client() {
                        Ok(c) => c.get_state().model.clone(),
                        Err(_) => "unknown".to_string(),
                    }
                ));
            }
            _ => {
                ui::report_error(&format!("Failed to switch model to: {}", args));
            }
        }
    }
}

pub async fn handle_provider_cmd(session: &mut ChatSession, args: &str) {
    if args.is_empty() {
        let active_providers = session.ctx.config_manager.get_active_providers();
        let current_provider = match session.get_client() {
            Ok(client) => client.get_state().provider.clone(),
            Err(e) => {
                ui::report_error(&e.to_string());
                return;
            }
        };
        ui::print_rule(Some("Active Providers"), Some("magenta"));
        for p in active_providers {
            if p == current_provider {
                println!("  {} {}", "●".magenta(), p.bold().magenta());
            } else {
                println!("    {}", p);
            }
        }
        ui::print_rule(None, Some("magenta"));
    } else {
        let (stdout, raw) = {
            let state = match session.get_client() {
                Ok(client) => client.get_state(),
                Err(e) => {
                    ui::report_error(&e.to_string());
                    return;
                }
            };
            (state.stdout, !state.render_markdown)
        };

        let client = {
            let registry = session.ctx.client_registry.lock().await;
            registry.create_client(args, "default", stdout, raw, &session.ctx.config_manager)
        };

        match client {
            Some(new_client) => {
                session.switch_client(new_client);
                let _ = session.ctx.config_manager.update_state(args, "default");
                ui::report_success(&format!("Switched to provider: {}", args));
            }
            _ => {
                ui::report_error(&format!("Unknown or inactive provider: {}", args));
            }
        }
    }
}

pub async fn handle_checkpoint(session: &mut ChatSession) {
    let history_len = match session.get_client() {
        Ok(client) => client.get_state().conversation.len(),
        Err(e) => {
            ui::report_error(&e.to_string());
            return;
        }
    };
    if history_len == 0 {
        ui::report_warning("History is empty. Nothing to checkpoint.");
        return;
    }

    ui::report_info("Creating checkpoint (summarizing history)...");

    let summary_prompt = "Please provide a concise but comprehensive summary of our work so far, including key findings, decisions made, and any pending tasks. This summary will be used as a 'checkpoint' message to replace the current conversation history and save context. Respond ONLY with the summary text.";

    let data = vec![DataSource {
        content: serde_json::Value::String(summary_prompt.to_string()),
        content_type: "text/plain".to_string(),
        is_file_or_url: false,
        metadata: std::collections::HashMap::new(),
    }];

    // Use process_and_print to get the summary so the user sees it and thinking is shown
    if let Err(e) = session.process_and_print(data).await {
        ui::report_error(&format!("Failed to create checkpoint: {}", e));
        return;
    }

    // After process_and_print, the history has [Old History] + [Summary Prompt] + [Summary Response]
    let state = match session.get_client_mut() {
        Ok(client) => client.get_state_mut(),
        Err(e) => {
            ui::report_error(&e.to_string());
            return;
        }
    };
    if let Some(last_msg) = state.conversation.last().cloned() {
        // Clear all history and replace with the summary
        state.conversation.clear();

        // Wrap the summary in a descriptive header
        let summary_text = last_msg.get_text(true);
        let checkpoint_msg = Message {
            role: Role::System,
            parts: vec![MessagePart::Text(format!(
                "--- CHECKPOINT SUMMARY ---\n{}\n--- END OF SUMMARY ---",
                summary_text
            ))],
        };
        state.conversation.push(checkpoint_msg);
        ui::report_success("Checkpoint created. History has been compressed.");
    }
}

pub fn print_help() {
    println!("\nChat Commands:");
    println!("  /help, /h       Show this help message");
    println!("  /quit, /q       Exit the application");
    println!("  /system [on|off] Show or toggle system prompt status");
    println!("  /edit, /e       Edit message in external editor");
    println!("  /clear, /c      Clear conversation history");
    println!("  /info, /i       Show session info");
    println!("  /debug          Toggle live debug mode");
    println!("  /raw            Show conversation as raw text");
    println!("  /dump           Dump conversation history as JSON");
    println!("  /save <path>    Save conversation history to JSON file");
    println!("  /load <path>    Load conversation history from JSON file");
    println!("  /attach <path>  Attach a file or URL to the next message");
    println!("  /tools [on|off] Show or toggle tool status");
    println!("  /model, /m      Switch models");
    println!("  /provider, /p   Switch provider");
    println!("  /checkpoint, /cp Summarize and compress history");
    println!();
}
