use crate::cli::ui;
use crate::core::session::ActiveSession;

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
