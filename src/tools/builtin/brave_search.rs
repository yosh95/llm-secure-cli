use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

use crate::config::models::AppConfig;

/// Performs a web search using the Brave Search LLM Context API.
///
/// Sends a GET request to `https://api.search.brave.com/res/v1/llm/context` with
/// the `X-Subscription-Token` header set to the `BRAVE_API_KEY` environment
/// variable. Returns pre-extracted web content (title, URL, snippets) optimized
/// for LLM consumption.
///
/// This tool is only registered when the `BRAVE_API_KEY` environment variable
/// is set at startup.
///
/// # Reliability
///
/// The Brave LLM Context API can be slow (5-30+ seconds) and occasionally
/// returns transient errors (429 Rate Limit, 503 Temporary Unavailable).
/// This implementation uses:
/// - A dedicated HTTP client with a 60-second timeout (vs 30s global default)
/// - Automatic retry with exponential backoff (up to 3 attempts) on 429/503
pub fn brave_search(
    args: HashMap<String, Value>,
    _config: Arc<AppConfig>,
) -> anyhow::Result<Value> {
    let query = match args.get("query") {
        Some(Value::String(s)) => s.clone(),
        Some(other) => {
            return Err(anyhow::anyhow!(
                "Invalid type for 'query': expected a string, got {other}",
            ));
        }
        None => {
            return Err(anyhow::anyhow!(
                "Missing 'query' argument. Provide the search query to submit to Brave Search.",
            ));
        }
    };

    if query.trim().is_empty() {
        return Err(anyhow::anyhow!("'query' must not be empty."));
    }

    let api_key = std::env::var("BRAVE_API_KEY").map_err(|_| {
        anyhow::anyhow!(
            "BRAVE_API_KEY environment variable is not set. Cannot perform Brave Search."
        )
    })?;

    // Use a dedicated HTTP client with a longer timeout (60 seconds)
    // because the Brave LLM Context API is significantly slower than
    // the standard search API and can exceed the global 30s timeout.
    let brave_client = crate::llm::base::create_http_client_with_timeout(60)
        .unwrap_or_else(|_| crate::utils::http::CLIENT.clone());

    // Clone query before moving into the closure
    let query_for_closure = query.clone();
    let api_key_for_closure = api_key.clone();

    // Retry configuration: up to 3 attempts with exponential backoff
    let max_retries = 3;
    let mut last_error: Option<anyhow::Error> = None;

    for attempt in 1..=max_retries {
        let query_attempt = query_for_closure.clone();
        let api_key_attempt = api_key_for_closure.clone();
        let client_attempt = brave_client.clone();

        let response_result = crate::core::session::run_cancellable(move || {
            let resp = client_attempt
                .get("https://api.search.brave.com/res/v1/llm/context")
                .header("X-Subscription-Token", &api_key_attempt)
                .header("Accept", "application/json")
                .query("q", &query_attempt)
                .call()
                .map_err(|e| {
                    let status = if let ureq::Error::StatusCode(s) = &e {
                        *s
                    } else {
                        0
                    };
                    anyhow::anyhow!("Brave Search request failed (HTTP {status}): {e}")
                })?;

            let text = resp
                .into_body()
                .read_to_string()
                .map_err(|e| anyhow::anyhow!("Failed to read Brave Search response body: {e}"))?;

            serde_json::from_str::<Value>(&text)
                .map_err(|e| anyhow::anyhow!("Failed to parse Brave Search JSON response: {e}"))
        });

        match response_result {
            Ok(response) => {
                // Success — extract results and return
                let mut results = Vec::new();
                if let Some(grounding) = response.get("grounding")
                    && let Some(items) = grounding.get("generic").and_then(|v| v.as_array())
                {
                    for item in items {
                        let title = item
                            .get("title")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default();
                        let url_str = item.get("url").and_then(|v| v.as_str()).unwrap_or_default();
                        let snippets: Vec<&str> = item
                            .get("snippets")
                            .and_then(|v| v.as_array())
                            .map(|arr| arr.iter().filter_map(|s| s.as_str()).collect())
                            .unwrap_or_default();

                        let hostname = response
                            .get("sources")
                            .and_then(|s| s.get(url_str))
                            .and_then(|s| s.get("hostname"))
                            .and_then(|v| v.as_str())
                            .unwrap_or_default();

                        results.push(json!({
                            "title": title,
                            "url": url_str,
                            "hostname": hostname,
                            "snippets": snippets,
                            "snippet_count": snippets.len(),
                        }));
                    }
                }

                return Ok(json!({
                    "query": query,
                    "results": results,
                    "result_count": results.len(),
                }));
            }
            Err(e) => {
                let is_retryable = e.to_string().contains("HTTP 429")
                    || e.to_string().contains("HTTP 503")
                    || e.to_string().contains("HTTP 502")
                    || e.to_string().contains("timed out")
                    || e.to_string().contains("timeout");
                if is_retryable && attempt < max_retries {
                    let delay_secs = 2u64.pow(attempt); // 2, 4, 8 seconds
                    tracing::warn!(
                        "Brave Search attempt {}/{} failed (retryable), retrying in {}s: {}",
                        attempt,
                        max_retries,
                        delay_secs,
                        e
                    );
                    std::thread::sleep(std::time::Duration::from_secs(delay_secs));
                    last_error = Some(e);
                    continue;
                }
                // Non-retryable or out of retries
                return Err(anyhow::anyhow!(
                    "Brave Search failed after {}/{} attempt(s): {}",
                    attempt,
                    max_retries,
                    e
                ));
            }
        }
    }

    Err(anyhow::anyhow!(
        "Brave Search failed after {} attempts. Last error: {}",
        max_retries,
        last_error.map_or_else(|| "unknown".to_string(), |e| e.to_string())
    ))
}
