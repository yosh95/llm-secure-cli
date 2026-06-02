use crate::cli::ui;
use crate::core::session::ActiveSession;
use colored::Colorize;

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
                    let prefix = "  \u{2500}\u{2500} ";
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
