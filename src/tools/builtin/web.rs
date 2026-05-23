use crate::config::models::AppConfig;

use crate::utils::http::CLIENT;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

/// Search the web using the Brave LLM Context API.
///
/// This uses Brave's LLM-optimized endpoint that returns pre-extracted content
/// (text, tables, code) ready for LLM consumption — no scraping needed.
pub async fn brave_search(
    args: HashMap<String, Value>,
    config: Arc<AppConfig>,
    api_key: &str,
) -> anyhow::Result<Value> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'query' is required"))?;

    let bc = &config.brave_search;

    let result = call_brave_llm_context(
        BraveLlmContextParams {
            query,
            count: bc.count,
            max_tokens: bc.max_tokens,
            max_urls: bc.max_urls,
            context_threshold_mode: &bc.context_threshold_mode,
            freshness: &bc.freshness,
        },
        api_key,
    )
    .await?;

    Ok(result)
}

/// Parameters for the Brave LLM Context API.
struct BraveLlmContextParams<'a> {
    query: &'a str,
    count: u64,
    max_tokens: u64,
    max_urls: u64,
    context_threshold_mode: &'a str,
    freshness: &'a str,
}

/// Call the Brave LLM Context API (`/res/v1/llm/context`).
///
/// Returns pre-extracted web content optimised for LLM consumption, including
/// grounded snippets from relevant pages along with source metadata.
async fn call_brave_llm_context(
    params: BraveLlmContextParams<'_>,
    api_key: &str,
) -> anyhow::Result<Value> {
    let mut body = serde_json::Map::new();
    body.insert("q".to_string(), json!(params.query));
    body.insert("count".to_string(), json!(params.count));
    body.insert(
        "maximum_number_of_tokens".to_string(),
        json!(params.max_tokens),
    );
    body.insert("maximum_number_of_urls".to_string(), json!(params.max_urls));

    if !params.context_threshold_mode.is_empty() {
        body.insert(
            "context_threshold_mode".to_string(),
            json!(params.context_threshold_mode),
        );
    }
    if !params.freshness.is_empty() {
        body.insert("freshness".to_string(), json!(params.freshness));
    }

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

    let data: Value = res.json().await?;

    let mut grounding_entries = Vec::new();

    if let Some(generic) = data
        .get("grounding")
        .and_then(|g| g.get("generic"))
        .and_then(|g| g.as_array())
    {
        for entry in generic {
            let e_url = entry
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let e_title = entry
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let snippets = entry
                .get("snippets")
                .and_then(|s| s.as_array())
                .cloned()
                .unwrap_or_default();

            grounding_entries.push(json!({
                "url": e_url,
                "title": e_title,
                "snippets": snippets,
            }));
        }
    }

    let poi = data.get("grounding").and_then(|g| g.get("poi"));
    let map_results = data
        .get("grounding")
        .and_then(|g| g.get("map"))
        .and_then(|m| m.as_array())
        .cloned();

    let sources = data.get("sources").cloned().unwrap_or(json!({}));

    let mut result = json!({
        "query": params.query,
        "count": grounding_entries.len(),
        "results": grounding_entries,
        "sources": sources,
    });

    if let Some(poi_val) = poi {
        result["poi"] = poi_val.clone();
    }
    if let Some(map_val) = map_results {
        result["map"] = Value::Array(map_val);
    }

    Ok(result)
}
