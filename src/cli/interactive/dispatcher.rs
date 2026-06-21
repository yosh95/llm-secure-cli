use crate::cli::interactive::model_handler;
use crate::cli::interactive::session_handler;
use crate::cli::ui;
use crate::core::session::ActiveSession;

pub use session_handler::handle_dump;
pub use session_handler::handle_edit_history;
pub use session_handler::handle_info;
pub use session_handler::handle_session_cmd;

pub enum CommandResult {
    Handled,
    NotACommand,
    Exit,
    Input(String),
}

pub fn handle_command(session: &mut ActiveSession, input: &str) -> CommandResult {
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
        "dump" => {
            session_handler::handle_dump(session);
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
        "model" | "m" => {
            model_handler::handle_model_cmd(session, args);
            CommandResult::Handled
        }
        "verifier" | "v" => {
            model_handler::handle_verifier_cmd(session, args);
            CommandResult::Handled
        }
        "credits" => {
            crate::cli::commands::credits::run_credits_interactive(session);
            CommandResult::Handled
        }
        "rankings" => {
            crate::cli::commands::rankings::run_rankings_interactive(session);
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

fn print_help() {
    ui::print_rule(Some("Interactive Commands"), Some("cyan"));
    println!("  /h, /help          Show this help message");
    println!("  /q, /quit          Exit the session");
    println!("  /i, /info          Show session and security status");
    println!("  /c, /clear         Clear conversation history");
    println!(
        "  /eh, /edit_history View/edit the conversation history in TOML format (includes full structure)"
    );
    println!("  /dump              Dump the conversation history as TOML to stdout");
    println!(
        "  /session [load|delete <id>|clear]  List, load, delete, or clear saved sessions (\"last\" for most recent)"
    );
    println!(
        "  /m, /model [-u] [<name>]  List models (/model -u to refresh ALL providers cache) or switch to provider:model"
    );
    println!(
        "  /v, /verifier [add|delete <provider:model>|list]  Add/delete/list verifier committee members"
    );
    println!(
        "  /credits           Show detailed OpenRouter credit info (uses both /credits and /key APIs)"
    );
    println!("  /rankings          Show OpenRouter model rankings (token usage leaderboard)");
    println!("  F2                 Open external editor to edit the current prompt (multi-line)");
    ui::print_rule(None, Some("cyan"));
}
