use crate::cli::ui;
use crate::config::ConfigManager;
use crate::core::session::ActiveSession;
use crate::utils::http;
use serde_json::Value;

/// Run the `credits` subcommand (CLI subcommand: `llsc credits`).
///
/// Fetches and displays the `OpenRouter` credit balance via the
/// `/api/v1/credits` and `/api/v1/key` endpoints.
/// Only works when the provider is `"openrouter"`.
pub fn run_credits(config_manager: &ConfigManager, provider: &str) {
    // Validate that the provider is "openrouter"
    if provider != "openrouter" {
        ui::report_error(&format!(
            "Credits check is only supported for the 'openrouter' provider, got '{provider}'."
        ));
        return;
    }

    // Get the API key
    let api_key = if let Some(key) = config_manager.get_api_key("openrouter") {
        key
    } else {
        ui::report_error(
            "OpenRouter API key is not configured. Set OPENROUTER_API_KEY in your environment or add it to config.toml under [providers.openrouter].",
        );
        return;
    };

    fetch_and_display_credits(&api_key);
}

/// Run the `/credits` command from the interactive session.
///
/// Uses the session's current provider and config.
pub fn run_credits_interactive(session: &ActiveSession) {
    let provider = session.get_client().get_state().provider.clone();

    if provider != "openrouter" {
        ui::report_error(&format!(
            "Credits check is only supported for the 'openrouter' provider, current provider is '{provider}'."
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

    fetch_and_display_credits(&api_key);
}

/// Common implementation: fetch from both /credits and /key APIs and display.
fn fetch_and_display_credits(api_key: &str) {
    let mut headers = std::collections::HashMap::new();
    headers.insert("Authorization".to_string(), format!("Bearer {api_key}"));

    ui::report_info("Fetching OpenRouter credits...");

    let credits_result = fetch_credits(&headers);
    let key_result = fetch_key_info(&headers);

    println!("OpenRouter Credits");

    // ---- Section 1: Credits API ----
    match credits_result {
        Ok(data) => {
            let total_credits = data
                .get("total_credits")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0);
            let total_usage = data
                .get("total_usage")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0);
            let remaining = total_credits - total_usage;
            let usage_pct = if total_credits > 0.0 {
                (total_usage / total_credits) * 100.0
            } else {
                0.0
            };

            println!();
            println!("  📊 Balance (Credits API)");
            println!("  {:<24} ${:<8.2}", "Total Purchased:", total_credits);
            println!("  {:<24} ${:<8.2}", "Total Used:", total_usage);
            println!("  {:<24} ${:<8.2}", "Remaining:", remaining);
            // Progress bar for usage
            let bar_width = 30;
            let filled = ((usage_pct / 100.0) * bar_width as f64).round() as usize;
            let filled = filled.min(bar_width);
            let bar = format!(
                "{}{}",
                "█".repeat(filled),
                "░".repeat(bar_width.saturating_sub(filled))
            );
            println!("  {:<24} {} {:5.1}%", "Usage:", bar, usage_pct);
        }
        Err(e) => {
            ui::report_error(&format!("Failed to fetch credits API: {e}"));
        }
    }

    // ---- Section 2: Key API ----
    match key_result {
        Ok(data) => {
            let label = data.get("label").and_then(|v| v.as_str()).unwrap_or("-");
            let limit = data.get("limit").and_then(serde_json::Value::as_f64);
            let limit_remaining = data
                .get("limit_remaining")
                .and_then(serde_json::Value::as_f64);
            let usage = data
                .get("usage")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0);
            let usage_daily = data
                .get("usage_daily")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0);
            let usage_weekly = data
                .get("usage_weekly")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0);
            let usage_monthly = data
                .get("usage_monthly")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0);
            let byok_usage = data
                .get("byok_usage")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0);
            let byok_usage_daily = data
                .get("byok_usage_daily")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0);
            let byok_usage_weekly = data
                .get("byok_usage_weekly")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0);
            let byok_usage_monthly = data
                .get("byok_usage_monthly")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0);
            let is_free_tier = data
                .get("is_free_tier")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            let limit_reset = data
                .get("limit_reset")
                .and_then(|v| v.as_str())
                .unwrap_or("never");

            println!();
            println!("  🔑 Key Info (Key API)");
            println!("  {:<24} {}", "Label:", label);
            println!(
                "  {:<24} {}",
                "Free Tier:",
                if is_free_tier { "Yes" } else { "No" }
            );

            // Key limit info
            if let Some(l) = limit {
                let remaining_str =
                    limit_remaining.map_or_else(|| "N/A".to_string(), |r| format!("${r:<.2}"));
                println!(
                    "  {:<24} ${:<8.2}  (remaining: {})",
                    "Key Limit:", l, remaining_str
                );
                if let Some(r) = limit_remaining {
                    let used_in_limit = l - r;
                    let pct = if l > 0.0 {
                        (used_in_limit / l) * 100.0
                    } else {
                        0.0
                    };
                    let bar_width = 20;
                    let filled = ((pct / 100.0) * bar_width as f64).round() as usize;
                    let filled = filled.min(bar_width);
                    let bar = format!(
                        "{}{}",
                        "█".repeat(filled),
                        "░".repeat(bar_width.saturating_sub(filled))
                    );
                    println!("  {:<24} {} {:5.1}%", "Limit Usage:", bar, pct);
                }
                println!("  {:<24} {}", "Limit Reset:", limit_reset);
            } else {
                println!("  {:<24} Unlimited", "Key Limit:");
            }

            println!();
            println!("  📈 Usage Breakdown (Key API)");
            println!("  {:<24} ${:<8.2}  (all time)", "Usage:", usage);
            println!(
                "  {:<24} ${:<8.2}  (current UTC day)",
                "Daily:", usage_daily
            );
            println!(
                "  {:<24} ${:<8.2}  (current UTC week)",
                "Weekly:", usage_weekly
            );
            println!(
                "  {:<24} ${:<8.2}  (current UTC month)",
                "Monthly:", usage_monthly
            );

            // BYOK usage (if any)
            if byok_usage > 0.0
                || byok_usage_daily > 0.0
                || byok_usage_weekly > 0.0
                || byok_usage_monthly > 0.0
            {
                println!();
                println!("  🔌 BYOK Usage (Bring Your Own Key)");
                println!("  {:<24} ${:<8.2}  (all time)", "BYOK Total:", byok_usage);
                println!(
                    "  {:<24} ${:<8.2}  (daily)",
                    "BYOK Daily:", byok_usage_daily
                );
                println!(
                    "  {:<24} ${:<8.2}  (weekly)",
                    "BYOK Weekly:", byok_usage_weekly
                );
                println!(
                    "  {:<24} ${:<8.2}  (monthly)",
                    "BYOK Monthly:", byok_usage_monthly
                );
            }
        }
        Err(e) => {
            ui::report_error(&format!("Failed to fetch key API: {e}"));
        }
    }
}

/// Fetch the /api/v1/credits endpoint.
fn fetch_credits(headers: &std::collections::HashMap<String, String>) -> Result<Value, String> {
    let url = "https://openrouter.ai/api/v1/credits".to_string();
    match http::get_json::<Value>(url, headers.clone()) {
        Ok(response) => match response.get("data") {
            Some(data) => Ok(data.clone()),
            None => Err(response
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Unexpected response format")
                .to_string()),
        },
        Err(e) => Err(format!("{e}")),
    }
}

/// Fetch the /api/v1/key endpoint.
fn fetch_key_info(headers: &std::collections::HashMap<String, String>) -> Result<Value, String> {
    let url = "https://openrouter.ai/api/v1/key".to_string();
    match http::get_json::<Value>(url, headers.clone()) {
        Ok(response) => match response.get("data") {
            Some(data) => Ok(data.clone()),
            None => Err(response
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Unexpected response format")
                .to_string()),
        },
        Err(e) => Err(format!("{e}")),
    }
}
