use serde_json::{Value, json};
use std::collections::HashMap;

/// Search the web using the Brave LLM Context API.
///
/// This uses Brave's LLM-optimized endpoint that returns pre-extracted content
/// (text, tables, code) ready for LLM consumption — no scraping needed.
///
/// The raw API response is returned as-is without any restructuring.
/// All optional parameters (count, `max_tokens`, `max_urls`, freshness, etc.) use API-side defaults.
pub fn brave_search(args: HashMap<String, Value>, api_key: &str) -> anyhow::Result<Value> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'query' is required"))?;

    call_brave_llm_context(query, api_key)
}

/// Call the Brave LLM Context API (`/res/v1/llm/context`).
///
/// Returns the raw JSON response from the API without any restructuring.
/// Only sends the query parameter; all other parameters use API-side defaults.
/// Supports Ctrl+C cancellation via [`run_cancellable`].
fn call_brave_llm_context(query: &str, api_key: &str) -> anyhow::Result<Value> {
    let mut body = serde_json::Map::new();
    body.insert("q".to_string(), json!(query));

    let url = "https://api.search.brave.com/res/v1/llm/context";
    let api_key = api_key.to_string();

    crate::core::session::run_cancellable(move || {
        let body_json = serde_json::Value::Object(body);
        let body_string = serde_json::to_string(&body_json)
            .map_err(|e| anyhow::anyhow!("Failed to serialize request body: {e}"))?;

        let response = crate::utils::http::CLIENT
            .post(url)
            .header("Accept", "application/json")
            .header("X-Subscription-Token", &api_key)
            .header("Content-Type", "application/json")
            .send(body_string)
            .map_err(|e| {
                if matches!(e, ureq::Error::Timeout(_)) {
                    anyhow::anyhow!("Brave LLM Context API timed out after 30s")
                } else {
                    anyhow::anyhow!("Brave LLM Context API request failed: {e}")
                }
            })?;

        let status = response.status().as_u16();
        let text = response
            .into_body()
            .read_to_string()
            .map_err(|e| anyhow::anyhow!("Failed to read response body: {e}"))?;

        if !(200..300).contains(&status) {
            let error_body = serde_json::from_str::<Value>(&text)
                .map(|v| v.to_string())
                .unwrap_or_else(|_| "Unable to read error response body".to_string());
            return Err(anyhow::anyhow!(
                "Brave LLM Context API error ({status}): {error_body}"
            ));
        }

        let data: Value = serde_json::from_str(&text)
            .map_err(|e| anyhow::anyhow!("Failed to parse response JSON: {e}"))?;
        Ok(data)
    })
}
