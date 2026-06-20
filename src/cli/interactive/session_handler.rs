use crate::cli::ui;
use crate::core::session::ActiveSession;
use crate::llm::models::{Message, MessagePart};
use jiff::{Timestamp, tz::TimeZone};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct ConversationDump {
    messages: Vec<Message>,
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
                            "unknown".to_string()
                        } else {
                            s.created_at
                                .parse::<Timestamp>()
                                .map_or_else(
                                    |_| s.created_at.clone(),
                                    |ts| {
                                        ts.to_zoned(TimeZone::system())
                                            .strftime("%Y-%m-%d %H:%M")
                                            .to_string()
                                    },
                                )
                                .to_string()
                        };
                        let first = s.first_user_prompt.as_deref().unwrap_or("(no user prompt)");
                        println!("  {}  {: <36} {}", ts, s.filename, first);
                    }
                    ui::print_rule(None, Some("cyan"));
                    println!(
                        "Usage: /session load|delete <id>  or  /session clear  (use \"last\" for most recent)"
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

pub fn handle_info(session: &ActiveSession) {
    let state = session.get_client().get_state();
    let _config = match session.ctx.config_manager.get_config() {
        Ok(c) => c,
        Err(e) => {
            ui::report_error(&format!("Failed to load config: {e}"));
            return;
        }
    };

    ui::print_rule(Some("Session Info"), Some("cyan"));
    ui::print_key_value("Session ID", &session.trace_id);
    ui::print_key_value(
        "Provider:Model",
        &format!("{}:{}", state.provider, state.model),
    );

    // Validator Info
    let (members, _) = session.ctx.config_manager.get_verifier_committee();
    let v_enabled = session.ctx.config_manager.get_verifier_enabled();
    let app_state = session.ctx.config_manager.get_state().ok();
    let runtime_members: Vec<String> = app_state.map(|s| s.verifier_committee).unwrap_or_default();

    if members.is_empty() {
        let status = if v_enabled {
            "NOT SET (Falling back to manual approval)".to_string()
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
        ui::print_key_value(label, &format!("{count} member(s)"));

        // Show runtime (state.toml) members with a marker
        let runtime_set: std::collections::HashSet<&str> =
            runtime_members.iter().map(|s| s.as_str()).collect();

        for (i, (p, m)) in members.iter().enumerate() {
            let pm_str = format!("{p}:{m}");
            let source_marker = if runtime_set.contains(pm_str.as_str()) {
                " (state.toml)".to_string()
            } else {
                " (config.toml)".to_string()
            };
            ui::print_key_value(
                &format!("  Member {}", i + 1),
                &format!("{p}:{m}{source_marker}"),
            );
        }
    }
    let v_status = if v_enabled {
        "ENABLED".to_string()
    } else {
        "DISABLED".to_string()
    };
    ui::print_key_value("Verifier Status", &v_status);

    // Tool Output is always displayed
    ui::print_key_value("Tool Output", "Always Visible");

    // Security
    ui::print_key_value("Security Level", "high");

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
            crate::llm::models::Role::Assistant | crate::llm::models::Role::Model => &state.model,
            crate::llm::models::Role::User => "USER",
            crate::llm::models::Role::System => "SYSTEM",
            crate::llm::models::Role::Tool => "TOOL",
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
