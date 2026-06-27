use crate::cli::ui;
use crate::config::ConfigManager;
use crate::core::session::ActiveSession;
use crate::utils::http;
use serde_json::Value;

/// Run the `rankings` subcommand (CLI subcommand: `llsc rankings`).
///
/// Fetches and displays the OpenRouter model rankings via the
/// `/api/v1/datasets/rankings-daily` endpoint.
/// Only works when the provider is `"openrouter"`.
pub fn run_rankings(config_manager: &ConfigManager, provider: &str) {
    if provider != "openrouter" {
        ui::report_error(&format!(
            "Rankings are only supported for the 'openrouter' provider, got '{provider}'."
        ));
        return;
    }

    let api_key = if let Some(key) = config_manager.get_api_key("openrouter") {
        key
    } else {
        ui::report_error(
            "OpenRouter API key is not configured. Set OPENROUTER_API_KEY in your environment or add it to config.toml under [providers.openrouter].",
        );
        return;
    };

    fetch_and_display_rankings(&api_key, None);
}

/// Run the `/rankings` command from the interactive session.
pub fn run_rankings_interactive(session: &ActiveSession) {
    let provider = session.get_client().get_state().provider.clone();

    if provider != "openrouter" {
        ui::report_error(&format!(
            "Rankings are only supported for the 'openrouter' provider, current provider is '{provider}'."
        ));
        return;
    }

    let api_key = if let Some(key) = session.ctx.config_manager.get_api_key("openrouter") {
        key
    } else {
        ui::report_error(
            "OpenRouter API key is not configured. Set OPENROUTER_API_KEY in your environment or add it to config.toml under [providers.openrouter].",
        );
        return;
    };

    fetch_and_display_rankings(&api_key, None);
}

/// Common implementation: fetch from the rankings-daily API and display.
///
/// `top_n` controls how many models to show (default: 15, capped at 50).
fn fetch_and_display_rankings(api_key: &str, top_n: Option<usize>) {
    let mut headers = std::collections::HashMap::new();
    headers.insert("Authorization".to_string(), format!("Bearer {api_key}"));

    ui::report_info("Fetching OpenRouter model rankings...");

    match fetch_rankings(&headers) {
        Ok(response) => {
            display_rankings(&response, top_n.unwrap_or(15));
        }
        Err(e) => {
            ui::report_error(&format!("Failed to fetch rankings: {e}"));
        }
    }
}

/// Fetch the /api/v1/datasets/rankings-daily endpoint.
fn fetch_rankings(headers: &std::collections::HashMap<String, String>) -> Result<Value, String> {
    let url = "https://openrouter.ai/api/v1/datasets/rankings-daily".to_string();
    match http::get_json::<Value>(url, headers.clone()) {
        Ok(response) => {
            if response.get("data").is_some() {
                Ok(response)
            } else {
                Err(response
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unexpected response format")
                    .to_string())
            }
        }
        Err(e) => Err(format!("{e}")),
    }
}

/// Display rankings in a formatted table.
fn display_rankings(response: &Value, top_n: usize) {
    let meta = response.get("meta").and_then(|m| m.as_object());
    let as_of = meta
        .and_then(|m| m.get("as_of").and_then(|v| v.as_str()))
        .unwrap_or("unknown");
    let end_date = meta
        .and_then(|m| m.get("end_date").and_then(|v| v.as_str()))
        .unwrap_or("unknown");
    let start_date = meta
        .and_then(|m| m.get("start_date").and_then(|v| v.as_str()))
        .unwrap_or("unknown");

    let data = match response.get("data").and_then(|d| d.as_array()) {
        Some(arr) => arr,
        None => {
            ui::report_error("No ranking data available.");
            return;
        }
    };

    // Group by date and pick the latest complete day
    let mut by_date: std::collections::BTreeMap<&str, Vec<&Value>> =
        std::collections::BTreeMap::new();
    for entry in data {
        if let Some(date) = entry.get("date").and_then(|v| v.as_str()) {
            by_date.entry(date).or_default().push(entry);
        }
    }

    let latest_date = by_date.keys().next_back().copied().unwrap_or("unknown");
    let latest = by_date
        .get(latest_date)
        .map(|v| v.as_slice())
        .unwrap_or_default();

    // Separate "other" (aggregated long-tail) from top models
    let models: Vec<&&Value> = latest
        .iter()
        .filter(|e| {
            e.get("model_permaslug")
                .and_then(|v| v.as_str())
                .is_some_and(|s| s != "other")
        })
        .collect();

    let other_entry = latest
        .iter()
        .find(|e| e.get("model_permaslug").and_then(|v| v.as_str()) == Some("other"));

    let total_tokens_all: u64 = models
        .iter()
        .filter_map(|e| e.get("total_tokens").and_then(|v| v.as_str()))
        .filter_map(|s| s.parse::<u64>().ok())
        .sum();

    let other_tokens: u64 = other_entry
        .and_then(|e| e.get("total_tokens").and_then(|v| v.as_str()))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    let grand_total = total_tokens_all + other_tokens;

    // ============ Display ============

    println!("OpenRouter Model Rankings");
    println!();
    println!("  Period:  {start_date}  \u{2192}  {end_date}");
    println!("  As of:  {}", as_of);
    println!(
        "  Showing:  Top {} Models ({} total tokens tracked)",
        top_n.min(models.len()),
        format_tokens(grand_total)
    );
    println!();

    // Header row
    println!(
        "  {:>4}  {:<48}  {:>16}  {:>7}",
        "Rank", "Model", "Weekly Tokens", "Share"
    );

    let display_count = top_n.min(models.len());
    for (i, entry) in models.iter().enumerate().take(display_count) {
        let rank = i + 1;
        let slug = entry
            .get("model_permaslug")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let tokens_str = entry
            .get("total_tokens")
            .and_then(|v| v.as_str())
            .unwrap_or("0");
        let tokens: u64 = tokens_str.parse().unwrap_or(0);
        let share = if grand_total > 0 {
            (tokens as f64 / grand_total as f64) * 100.0
        } else {
            0.0
        };

        // Truncate model name
        let display_name = if slug.len() > 47 {
            format!("{}\u{2026}", &slug[..46])
        } else {
            slug.to_string()
        };

        // Color by rank (right-aligned to keep columns aligned)
        let rank_visible = format!("#{rank}");
        let rank_str = format!("{:>3}", rank_visible).to_string();

        println!(
            "  {}  {:<48}  {:>16}  {:>6.1}%",
            rank_str,
            display_name,
            format_tokens(tokens),
            share,
        );
    }

    // "Other" (long tail) row
    if other_tokens > 0 {
        let other_share = (other_tokens as f64 / grand_total as f64) * 100.0;
        println!(
            "       {:<48}  {:>16}  {:>6.1}%",
            "(other models)".to_string(),
            format_tokens(other_tokens),
            other_share
        );
    }

    println!();
    println!(
        "Total: {} tokens across all models",
        format_tokens(grand_total)
    );

    // ============ Provider Breakdown ============
    let mut provider_tokens: std::collections::HashMap<&str, u64> =
        std::collections::HashMap::new();
    for entry in models.iter().take(display_count) {
        let slug = entry
            .get("model_permaslug")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let provider = slug.split('/').next().unwrap_or("unknown");
        let tokens_str = entry
            .get("total_tokens")
            .and_then(|v| v.as_str())
            .unwrap_or("0");
        let tokens: u64 = tokens_str.parse().unwrap_or(0);
        *provider_tokens.entry(provider).or_insert(0) += tokens;
    }

    println!();
    println!("Provider Breakdown");
    println!();

    // Collect into owned Vec to simplify type handling
    let mut sorted_providers: Vec<(&str, u64)> = provider_tokens.into_iter().collect();
    sorted_providers.sort_by_key(|b| std::cmp::Reverse(b.1));

    for (provider, token_count) in &sorted_providers {
        let share = if total_tokens_all > 0 {
            (*token_count as f64 / total_tokens_all as f64) * 100.0
        } else {
            0.0
        };
        println!(
            "  {:<20} {:>16} {:>6.1}%",
            provider,
            format_tokens(*token_count),
            share,
        );
    }

    // ============ Trend note ============
    println!();

    println!("  Rankings show real token usage across OpenRouter (top 50 models per day).");
    println!(
        "  Data range: {} records from {} to {}",
        data.len(),
        start_date,
        end_date
    );
}

/// Format large token counts into human-readable strings.
fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000_000_000 {
        format!("{:.2}T", tokens as f64 / 1_000_000_000_000.0)
    } else if tokens >= 1_000_000_000 {
        format!("{:.2}B", tokens as f64 / 1_000_000_000.0)
    } else if tokens >= 1_000_000 {
        format!("{:.2}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}K", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}
