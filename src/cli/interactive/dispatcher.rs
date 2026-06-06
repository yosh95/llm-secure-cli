use crate::cli::interactive::content_handler;
use crate::cli::interactive::model_handler;
use crate::cli::interactive::session_handler;
use crate::cli::interactive::tool_handler;
use crate::cli::ui;
use crate::core::session::ActiveSession;
use colored::Colorize;

pub use session_handler::handle_edit_history;
pub use session_handler::handle_info;
pub use session_handler::handle_raw;
pub use session_handler::handle_session_cmd;

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
            session_handler::handle_info(session);
            CommandResult::Handled
        }
        "raw" => {
            session_handler::handle_raw(session);
            CommandResult::Handled
        }
        "edit_history" | "eh" => {
            session_handler::handle_edit_history(session);
            CommandResult::Handled
        }
        "session" => {
            session_handler::handle_session_cmd(session, args);
            CommandResult::Handled
        }
        "attach" => {
            content_handler::handle_attach(session, args).await;
            CommandResult::Handled
        }
        "tools" => {
            tool_handler::handle_tools(session, args).await;
            CommandResult::Handled
        }
        "model" | "m" => {
            model_handler::handle_model_cmd(session, args).await;
            CommandResult::Handled
        }
        "summarize" | "s" => {
            content_handler::handle_summarize(session).await;
            CommandResult::Handled
        }
        "alias" => {
            model_handler::handle_alias_cmd(session, args).await;
            CommandResult::Handled
        }

        "t" | "template" => {
            return handle_template_cmd(session, args).await;
        }
        "view" => {
            content_handler::handle_view_cmd(session, args).await;
            CommandResult::Handled
        }
        "tool_output" | "to" => {
            tool_handler::handle_tool_output_cmd(session, args);
            CommandResult::Handled
        }
        "credits" => {
            crate::cli::commands::credits::run_credits_interactive(session).await;
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

async fn handle_template_cmd(session: &mut ActiveSession, args: &str) -> CommandResult {
    let templates = session.ctx.config_manager.load_templates();
    if args.is_empty() {
        if templates.is_empty() {
            ui::report_info("No templates found. Place .txt or .md files in ~/.llsc/templates/");
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
                                format!("{}...", trimmed.chars().take(60).collect::<String>())
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
        ui::report_success(&format!("Template '{args}' inserted into prompt."));
        CommandResult::Input(content.clone())
    } else {
        ui::report_error(&format!(
            "Template '{args}' not found. Use /t to list available templates."
        ));
        CommandResult::Handled
    }
}

fn print_help() {
    ui::print_rule(Some("Interactive Commands"), Some("cyan"));
    println!("  /h, /help          Show this help message");
    println!("  /q, /quit          Exit the session");
    println!("  /i, /info          Show session and security status");
    println!("  /c, /clear         Clear conversation history");
    println!(
        "  /eh, /edit_history View/edit the conversation history in TOML format (includes full structure)"
    );
    println!("  /session [load|delete <id>|clear]  List, load, delete, or clear saved sessions");
    println!("  /attach <path|url> Attach a file or URL to the next request");
    println!(
        "  /tools [on|off]    Toggle or show status of tool execution /to, /tool_output [on|off] Toggle display of tool execution results (default: hidden)"
    );
    println!(
        "  /m, /model [-u] [<name>]  List models (/model -u to refresh ALL providers cache) or switch to provider:model"
    );
    println!("  /alias [-d <name>] [<name> <target>]  List/create/delete model aliases");
    println!("  /s, /summarize     Summarize history and clear it");
    println!("  /t, /template [<name>]  List templates or insert one into prompt");
    println!(
        "  /view [<path>]     Open saved image or file with system default app (no arg = latest)"
    );
    println!(
        "  /credits           Show detailed OpenRouter credit info (uses both /credits and /key APIs)"
    );
    println!("  /raw               Show raw conversation history");
    println!("  F2                 Open external editor to edit the current prompt (multi-line)");
    ui::print_rule(None, Some("cyan"));
}
