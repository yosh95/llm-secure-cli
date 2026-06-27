use crate::cli::ui;
use crate::core::session::ActiveSession;

pub fn handle_model_cmd(session: &mut ActiveSession, args: &str) {
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
        session.ctx.config_manager.update_models_cache();
        ui::report_success("Models cache updated for all providers.");
        // fall through to list
    }

    // No args or -u: show all models grouped by provider, sorted
    if args_trimmed.is_empty() || args_trimmed == "-u" || args_trimmed == "--update" {
        println!("Available Models (provider:model)");
        let models_map = session.ctx.config_manager.get_cached_models();

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
                println!("  \u{25cf} {}", display);
            } else {
                println!("    {display}");
            }
        }

        if all_entries.is_empty() {
            println!("  No models cached. Use /model -u to fetch models now.");
        }
        return;
    }

    // `/model -i` or `/model --info [provider:model]`: show detailed model info (endpoints etc.)
    if args_trimmed == "-i"
        || args_trimmed == "--info"
        || args_trimmed.starts_with("-i ")
        || args_trimmed.starts_with("--info ")
    {
        let model_spec = if args_trimmed == "-i" || args_trimmed == "--info" {
            // Use current model if no argument
            format!("{}:{}", current_provider, current_model)
        } else {
            args_trimmed
                .strip_prefix("-i ")
                .or_else(|| args_trimmed.strip_prefix("--info "))
                .unwrap_or("")
                .trim()
                .to_string()
        };

        // Resolve provider:model
        let (ep_provider, ep_model) = if let Some((p, m)) = model_spec.split_once(':') {
            (p.to_string(), m.to_string())
        } else {
            (current_provider.clone(), model_spec)
        };

        handle_endpoints_cmd(&session.ctx.config_manager, &ep_provider, &ep_model);
        return;
    }

    // With argument: parse "provider:model" or just "model" (use current provider)
    let models_map = session.ctx.config_manager.get_cached_models();

    let resolved_provider: String;
    let resolved_model: String;

    if let Some((p, m)) = args_trimmed.split_once(':') {
        // provider:model format
        resolved_provider = p.to_string();
        resolved_model = m.to_string();
    } else {
        // Just model name - use current provider
        resolved_provider = current_provider.clone();
        resolved_model = args_trimmed.to_string();
    }

    // Validate: check the resolved provider has this model in cache
    let cached = models_map
        .get(&resolved_provider)
        .cloned()
        .unwrap_or_default();
    if !cached.contains(&resolved_model) {
        // Check if the arg is just a provider name (no colon)
        let active_providers = session
            .ctx
            .client_registry
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .list_providers();
        if !args_trimmed.contains(':') && active_providers.contains(&args_trimmed.to_string()) {
            return handle_provider_only_switch(session, args_trimmed);
        }
        ui::report_error(&format!(
            "Unknown model: '{}'. Use /model to list available models.",
            args_trimmed
        ));
        return;
    }

    match crate::core::initializer::switch_model(
        session,
        &resolved_model,
        &resolved_provider,
        stdout,
        !raw,
    ) {
        Ok(()) => {
            let state = session.get_client().get_state();
            ui::report_success(&format!("Switched to {}:{}", state.provider, state.model));
        }
        Err(e) => ui::report_error(&format!("Failed to switch: {e}")),
    }
}

/// Handle switching to a provider with its default model (no model specified).
fn handle_provider_only_switch(session: &mut ActiveSession, provider: &str) {
    match crate::core::initializer::switch_provider(session, provider) {
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

pub fn handle_verifier_cmd(session: &mut ActiveSession, args: &str) {
    let args_trimmed = args.trim();
    let config_manager = &session.ctx.config_manager;

    // `/verifier` (no args): list all verifier committee members
    if args_trimmed.is_empty() {
        let state = match config_manager.get_state() {
            Ok(s) => s,
            Err(e) => {
                ui::report_error(&format!("Failed to load state: {e}"));
                return;
            }
        };
        let (members, enabled) = config_manager.get_verifier_committee();
        let runtime_members = state.verifier_committee;

        if members.is_empty() {
            let status = if enabled {
                "Enabled (no members)"
            } else {
                "Disabled"
            };
            println!("Verifier Status: {}", status);
            println!("  No verifier committee members configured.");
            println!("  Usage: /verifier add <provider:model>");
            println!("         /verifier delete <provider:model>");
            println!("         /verifier list");
            return;
        }

        let status = if enabled { "ENABLED" } else { "DISABLED" };
        println!("Verifier Status: {}\n", status);

        println!("Verifier Committee Members");
        for (i, (p, m)) in members.iter().enumerate() {
            let pm_str = format!("{p}:{m}");
            // Mark if this member was set via state.toml (runtime) or config.toml (fallback)
            let source = if runtime_members.contains(&pm_str) {
                " (state.toml)".to_string()
            } else {
                " (config.toml)".to_string()
            };
            println!("  {}. {}  {}", i + 1, pm_str, source);
        }

        println!("Usage: /verifier add|delete <provider:model>");
        return;
    }

    let parts: Vec<&str> = args_trimmed.splitn(2, ' ').collect();
    let subcmd = parts[0];
    let subargs = if parts.len() > 1 { parts[1].trim() } else { "" };

    match subcmd {
        "add" => {
            if subargs.is_empty() {
                ui::report_error("Usage: /verifier add <provider:model>");
                return;
            }
            // Validate provider:model format
            if !subargs.contains(':') {
                ui::report_error("Invalid format. Use provider:model (e.g., ollama:gemma4:e2b)");
                return;
            }
            match config_manager.add_verifier_committee_member(subargs) {
                Ok(()) => {
                    ui::report_success(&format!(
                        "Verifier committee member '{}' added (persisted to state.toml).",
                        subargs
                    ));
                }
                Err(e) => ui::report_error(&format!("Failed to add verifier member: {e}")),
            }
        }
        "delete" | "del" | "remove" | "rm" => {
            if subargs.is_empty() {
                ui::report_error("Usage: /verifier delete <provider:model>");
                return;
            }
            match config_manager.remove_verifier_committee_member(subargs) {
                Ok(true) => {
                    ui::report_success(&format!(
                        "Verifier committee member '{}' removed (persisted to state.toml).",
                        subargs
                    ));
                }
                Ok(false) => {
                    ui::report_info(&format!(
                        "Verifier committee member '{}' not found.",
                        subargs
                    ));
                }
                Err(e) => ui::report_error(&format!("Failed to remove verifier member: {e}")),
            }
        }
        "list" | "ls" => {
            let (members, _enabled) = config_manager.get_verifier_committee();
            if members.is_empty() {
                ui::report_info("No verifier committee members configured.");
            } else {
                println!("Verifier Committee Members");
                for (i, (p, m)) in members.iter().enumerate() {
                    println!("  {}. {}:{}", i + 1, p, m);
                }
            }
        }
        _ => {
            ui::report_error("Usage: /verifier [add|delete <provider:model>|list]");
        }
    }
}

fn handle_endpoints_cmd(
    config_manager: &crate::config::ConfigManager,
    provider: &str,
    model: &str,
) {
    if provider != "openrouter" {
        ui::report_error(&format!(
            "Endpoints are only available for OpenRouter models. Current provider: '{provider}'."
        ));
        return;
    }

    let api_key = match config_manager.get_api_key("openrouter") {
        Some(key) => key,
        None => {
            ui::report_error(
                "OpenRouter API key is not configured. Set OPENROUTER_API_KEY in your environment.",
            );
            return;
        }
    };

    // The model slug for OpenRouter is like "openai/gpt-4o" — we need author and slug
    // If the model doesn't have a '/', assume it's just a slug and use provider as author
    let (author, slug) = if let Some((a, s)) = model.split_once('/') {
        (a.to_string(), s.to_string())
    } else {
        // If no '/' in the model, use the provider portion before ':' if it exists
        // But we already received provider and model separately, so model is the slug
        (provider.to_string(), model.to_string())
    };

    let url = format!(
        "https://openrouter.ai/api/v1/models/{}/{}/endpoints",
        urlencoding(&author),
        urlencoding(&slug),
    );

    ui::report_info(&format!("Fetching endpoints for {provider}:{model}..."));

    let mut headers = std::collections::HashMap::new();
    headers.insert("Authorization".to_string(), format!("Bearer {api_key}"));

    match crate::utils::http::get_json::<serde_json::Value>(url, headers) {
        Ok(json) => {
            let data = json.get("data").cloned().unwrap_or(json);
            display_model_endpoints(&data);
        }
        Err(e) => {
            // Fallback: try without /endpoints suffix
            let alt_url = format!(
                "https://openrouter.ai/api/v1/models/{}/{}",
                urlencoding(&author),
                urlencoding(&slug),
            );
            match crate::utils::http::get_json::<serde_json::Value>(
                alt_url,
                std::collections::HashMap::new(),
            ) {
                Ok(json) => {
                    let data = json.get("data").cloned().unwrap_or(json);
                    display_model_endpoints(&data);
                }
                Err(e2) => {
                    ui::report_error(&format!("Failed to fetch endpoints: {e} (alt: {e2})"));
                }
            }
        }
    }
}

/// URL-encode a string component for safe path usage.
fn urlencoding(s: &str) -> String {
    s.split('/')
        .map(|part| {
            part.chars()
                .map(|c| match c {
                    'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
                    _ => format!("%{:02X}", c as u8),
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Display the model endpoints data in a formatted way.
fn display_model_endpoints(data: &serde_json::Value) {
    let model_id = data.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
    let name = data
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(model_id);
    let description = data
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Model info
    println!("📋 {}", name);
    if !description.is_empty() {
        println!("  Description: {}", description);
    }
    println!("  Model ID: {}", model_id);

    // Architecture info
    if let Some(arch) = data.get("architecture") {
        let modality = arch
            .get("modality")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let instruct_type = arch
            .get("instruct_type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let tokenizer = arch
            .get("tokenizer")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let input_mods: Vec<&str> = arch
            .get("input_modalities")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        let output_mods: Vec<&str> = arch
            .get("output_modalities")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        println!("  Modality: {}", modality);
        println!("  Tokenizer: {}", tokenizer);
        println!("  Instruct Type: {}", instruct_type);
        println!("  Input Modalities: {}", input_mods.join(", "));
        println!("  Output Modalities: {}", output_mods.join(", "));
    }

    println!();

    // Endpoints
    if let Some(endpoints) = data.get("endpoints").and_then(|v| v.as_array()) {
        if endpoints.is_empty() {
            println!(
                "  No endpoint details available.
"
            );

            return;
        }

        println!(
            "  🔌 Providers — {} available endpoint(s)
",
            endpoints.len()
        );

        for (i, ep) in endpoints.iter().enumerate() {
            let _provider_name = ep
                .get("provider_name")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown");
            let ep_name = ep.get("name").and_then(|v| v.as_str()).unwrap_or("");

            // Status indicator
            let status = ep.get("status").and_then(|v| v.as_i64()).unwrap_or(-1);
            let status_str = match status {
                0 => "🟢 Online",
                1 => "🟡 Degraded",
                2 => "🔴 Offline",
                _ => "⚪ Unknown",
            };

            println!("  • {}. {} [{}]", i + 1, ep_name, status_str);

            // Pricing
            if let Some(pricing) = ep.get("pricing") {
                let prompt_str = pricing
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .unwrap_or("0");
                let completion_str = pricing
                    .get("completion")
                    .and_then(|v| v.as_str())
                    .unwrap_or("0");
                let image_str = pricing.get("image").and_then(|v| v.as_str()).unwrap_or("0");

                let prompt_per_m = format_usd_per_m(prompt_str);
                let completion_per_m = format_usd_per_m(completion_str);
                let image_per = format_usd(image_str);

                println!("     💰 Input:    {}", prompt_per_m);
                println!("     💰 Output:   {}", completion_per_m);
                if image_str != "0" {
                    println!("     🖼️ Image:    {}", image_per);
                }
            }

            // Context & output limits
            let ctx_len = ep
                .get("context_length")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let max_comp = ep.get("max_completion_tokens").and_then(|v| v.as_i64());
            let _max_prompt = ep.get("max_prompt_tokens").and_then(|v| v.as_i64());

            println!("     📐 Context:  {} tokens", format_number(ctx_len));
            if let Some(mc) = max_comp {
                println!("     📐 Max out:  {} tokens", format_number(mc));
            }

            // Quantization
            if let Some(quant) = ep.get("quantization").and_then(|v| v.as_str())
                && !quant.is_empty()
            {
                println!("     🔬 Quant:    {}", quant);
            }

            // Supported parameters
            if let Some(params) = ep.get("supported_parameters").and_then(|v| v.as_array()) {
                let param_names: Vec<&str> = params.iter().filter_map(|v| v.as_str()).collect();
                if !param_names.is_empty() {
                    println!("     ⚙️ Params:   {}", param_names.join(", "));
                }
            }

            // Caching
            let supports_caching = ep
                .get("supports_implicit_caching")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if supports_caching {
                println!("     💾 Caching:  ✅ Implicit caching supported");
            }

            // Performance metrics (latency & throughput)
            if let Some(latency) = ep.get("latency_last_30m") {
                let p50 = latency.get("p50").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let p90 = latency.get("p90").and_then(|v| v.as_f64()).unwrap_or(0.0);
                println!(
                    "     ⏱️ Latency:  p50={:.0}ms  p90={:.0}ms (last 30m)",
                    p50, p90
                );
            }
            if let Some(throughput) = ep.get("throughput_last_30m") {
                let p50 = throughput
                    .get("p50")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let p90 = throughput
                    .get("p90")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                println!(
                    "     🚀 Throughput: p50={:.0} t/s  p90={:.0} t/s (last 30m)",
                    p50, p90
                );
            }

            // Uptime
            let uptime_1d = ep.get("uptime_last_1d").and_then(|v| v.as_f64());
            let uptime_5m = ep.get("uptime_last_5m").and_then(|v| v.as_f64());
            if let Some(u) = uptime_5m {
                println!(
                    "     📊 Uptime:   {:5.1}% (5m)  {}% (1d)",
                    u,
                    uptime_1d.map_or("N/A".to_string(), |v| format!("{:5.1}", v))
                );
            }

            println!();
        }
    } else {
        // If no endpoints array, show the model-level pricing directly
        println!(
            "  No endpoint-level details. Showing model-level pricing:
"
        );
        if let Some(pricing) = data.get("pricing") {
            let prompt_str = pricing
                .get("prompt")
                .and_then(|v| v.as_str())
                .unwrap_or("0");
            let completion_str = pricing
                .get("completion")
                .and_then(|v| v.as_str())
                .unwrap_or("0");
            let image_str = pricing.get("image").and_then(|v| v.as_str()).unwrap_or("0");

            println!("     💰 Input:    {}", format_usd_per_m(prompt_str));
            println!("     💰 Output:   {}", format_usd_per_m(completion_str));
            if image_str != "0" {
                println!("     🖼️ Image:    {}", format_usd(image_str));
            }
        }

        let ctx_len = data
            .get("context_length")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        println!("     📐 Context:  {} tokens", format_number(ctx_len));

        // Supported parameters from model level
        if let Some(params) = data.get("supported_parameters").and_then(|v| v.as_array()) {
            let param_names: Vec<&str> = params.iter().filter_map(|v| v.as_str()).collect();
            if !param_names.is_empty() {
                println!("     ⚙️ Params:   {}", param_names.join(", "));
            }
        }
    }
}

/// Format a price string (USD per token) to a human-readable "$X.XX/1M tokens" format.
fn format_usd_per_m(price_str: &str) -> String {
    let price: f64 = price_str.parse().unwrap_or(0.0);
    let per_m = price * 1_000_000.0;
    format!("${:.4}/1M tokens", per_m)
}

/// Format a price as USD.
fn format_usd(price_str: &str) -> String {
    let price: f64 = price_str.parse().unwrap_or(0.0);
    if price == 0.0 {
        "Free".to_string()
    } else {
        format!("${:.2}", price)
    }
}

/// Format a large number with comma separators.
fn format_number(n: i64) -> String {
    let s = n.to_string();
    let mut result = String::new();
    let len = s.len();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(c);
    }
    result
}
