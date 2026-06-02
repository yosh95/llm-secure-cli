use crate::cli::ui;
use crate::core::session::ActiveSession;
use colored::Colorize;

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
