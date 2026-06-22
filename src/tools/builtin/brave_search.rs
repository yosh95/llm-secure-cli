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

    let client = &crate::utils::http::CLIENT;

    // Clone query before moving into the closure so we can use it later for the result
    let query_for_closure = query.clone();

    let response = crate::core::session::run_cancellable(move || {
        let resp = client
            .get("https://api.search.brave.com/res/v1/llm/context")
            .header("X-Subscription-Token", &api_key)
            .header("Accept", "application/json")
            .query("q", &query_for_closure)
            .call()
            .map_err(|e| anyhow::anyhow!("Brave Search LLM Context API request failed: {e}"))?;

        let text = resp
            .into_body()
            .read_to_string()
            .map_err(|e| anyhow::anyhow!("Failed to read Brave Search response body: {e}"))?;

        serde_json::from_str::<Value>(&text)
            .map_err(|e| anyhow::anyhow!("Failed to parse Brave Search JSON response: {e}"))
    })?;

    // Extract relevant fields from the LLM Context API response
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

            // Also include source metadata if available
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

    Ok(json!({
        "query": query,
        "results": results,
        "result_count": results.len(),
    }))
}
