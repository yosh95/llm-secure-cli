use crate::cli::ui;
use crate::core::session::ActiveSession;
use colored::Colorize;

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

pub async fn handle_model_cmd(session: &mut ActiveSession, args: &str) {
    let (current_provider, current_model, stdout, raw) = {
        let state = session.get_client().get_state();
        (
            state.provider.clone(),
            state.model.clone(),
            state.stdout,
            !state.render_markdown,
        )
    };

    let args_trimmed = args.trim();

    // `/model -u` or `/model --update`: refresh the models cache for ALL providers
    if args_trimmed == "-u" || args_trimmed == "--update" {
        ui::report_info("Updating models cache for all providers...");
        session.ctx.config_manager.update_models_cache().await;
        ui::report_success("Models cache updated for all providers.");
        // fall through to list
    }

    // No args or -u: show all models grouped by provider, sorted
    if args_trimmed.is_empty() || args_trimmed == "-u" || args_trimmed == "--update" {
        ui::print_rule(Some("Available Models (provider:model)"), Some("cyan"));
        let models_map = session.ctx.config_manager.get_cached_models().await;

        // Collect all provider:model pairs and sort them
        let mut all_entries: Vec<(String, String, bool)> = Vec::new();
        let mut providers: Vec<&String> = models_map.keys().collect();
        providers.sort();
        for p in &providers {
            if let Some(models) = models_map.get(*p) {
                let mut sorted_models = models.clone();
                sorted_models.sort();
                for m in sorted_models {
                    let is_current = *p == &current_provider && m == current_model;
                    all_entries.push(((*p).clone(), m, is_current));
                }
            }
        }

        for (p, m, is_current) in &all_entries {
            let display = format!("{p}:{m}");
            if *is_current {
                println!("  {} {}", "\u{25cf}".cyan(), display.bold().cyan());
            } else {
                println!("    {display}");
            }
        }

        if all_entries.is_empty() {
            println!("  No models cached. Use /model -u to fetch models now.");
        }

        // Show aliases
        let state = match session.ctx.config_manager.get_state() {
            Ok(s) => s,
            Err(_) => return,
        };
        if !state.model_aliases.is_empty() {
            println!("\nConfigured Aliases:");
            let mut aliases: Vec<_> = state.model_aliases.iter().collect();
            aliases.sort_by_key(|(k, _)| *k);
            for (name, alias) in aliases {
                println!(
                    "  {}    {} -> {}",
                    "\u{25cf}".cyan(),
                    name.bold().cyan(),
                    alias.target
                );
            }
        }
        return;
    }

    // With argument: parse "provider:model" or just "model" (use current provider)
    // Also check aliases first
    let state = match session.ctx.config_manager.get_state() {
        Ok(s) => s,
        Err(_) => return,
    };
    let models_map = session.ctx.config_manager.get_cached_models().await;

    let resolved_provider: String;
    let resolved_model: String;

    if state.model_aliases.contains_key(args_trimmed) {
        // Alias resolution
        let alias = &state.model_aliases[args_trimmed];
        if let Some((p, m)) = alias.target.split_once(':') {
            resolved_provider = p.to_string();
            resolved_model = m.to_string();
        } else {
            resolved_provider = current_provider.clone();
            resolved_model = alias.target.clone();
        }
    } else if let Some((p, m)) = args_trimmed.split_once(':') {
        // provider:model format
        resolved_provider = p.to_string();
        resolved_model = m.to_string();
    } else {
        // Just model name - use current provider
        resolved_provider = current_provider.clone();
        resolved_model = args_trimmed.to_string();
    }

    // Validate: check the resolved provider has this model in cache (unless alias)
    if !state.model_aliases.contains_key(args_trimmed) {
        let cached = models_map
            .get(&resolved_provider)
            .cloned()
            .unwrap_or_default();
        if !cached.contains(&resolved_model) {
            // Check if the arg is just a provider name (no colon)
            let active_providers = session.ctx.client_registry.lock().await.list_providers();
            if !args_trimmed.contains(':') && active_providers.contains(&args_trimmed.to_string()) {
                return handle_provider_only_switch(session, args_trimmed).await;
            }
            ui::report_error(&format!(
                "Unknown model: '{}'. Use /model to list available models.",
                args_trimmed
            ));
            return;
        }
    }

    match crate::core::initializer::switch_model(
        session,
        &resolved_model,
        &resolved_provider,
        stdout,
        !raw,
    )
    .await
    {
        Ok(()) => {
            let state = session.get_client().get_state();
            ui::report_success(&format!("Switched to {}:{}", state.provider, state.model));
        }
        Err(e) => ui::report_error(&format!("Failed to switch: {e}")),
    }
}

/// Handle switching to a provider with its default model (no model specified).
async fn handle_provider_only_switch(session: &mut ActiveSession, provider: &str) {
    match crate::core::initializer::switch_provider(session, provider).await {
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
