use crate::utils::http::CLIENT;
use serde_json::{Value, json};
use std::collections::HashMap;

/// Search the web using the Brave LLM Context API.
///
/// This uses Brave's LLM-optimized endpoint that returns pre-extracted content
/// (text, tables, code) ready for LLM consumption — no scraping needed.
///
/// The raw API response is returned as-is without any restructuring.
/// All optional parameters (count, max_tokens, max_urls, freshness, etc.) use API-side defaults.
pub async fn brave_search(
    args: HashMap<String, Value>,
    api_key: &str,
) -> anyhow::Result<Value> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'query' is required"))?;

    call_brave_llm_context(query, api_key).await
}

/// Call the Brave LLM Context API (`/res/v1/llm/context`).
///
/// Returns the raw JSON response from the API without any restructuring.
/// Only sends the query parameter; all other parameters use API-side defaults.
async fn call_brave_llm_context(
    query: &str,
    api_key: &str,
) -> anyhow::Result<Value> {
    let mut body = serde_json::Map::new();
    body.insert("q".to_string(), json!(query));

    let url = "https://api.search.brave.com/res/v1/llm/context";

    let res = CLIENT
        .post(url)
        .header("Accept", "application/json")
        .header("X-Subscription-Token", api_key)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;

    let status = res.status();
    if !status.is_success() {
        let error_body = match res.json::<Value>().await {
            Ok(v) => v.to_string(),
            Err(_) => "Unable to read error response body".to_string(),
        };
        return Err(anyhow::anyhow!(
            "Brave LLM Context API error ({}): {}",
            status,
            error_body
        ));
    }

    // Return the raw API response as-is without restructuring
    let data: Value = res.json().await?;
    Ok(data)
}
