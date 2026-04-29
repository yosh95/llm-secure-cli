use crate::config::models::AppConfig;
use crate::consts::{MAX_OUTPUT_CHARS, MAX_OUTPUT_LINES};
use crate::utils::http::CLIENT;
use serde_json::{Value, json};
use std::collections::HashMap;

/// Fetch a web page URL or PDF URL and convert the content to Markdown/text.
pub async fn read_url_content(
    args: HashMap<String, Value>,
    _config: AppConfig,
) -> anyhow::Result<Value> {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'url' is required"))?;

    let start_line = args
        .get("start_line")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(1)
        .max(1);

    let end_line = args
        .get("end_line")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);

    // Validate URL (block SSRF: private/loopback addresses)
    validate_url_ssrf(url)?;

    let content = fetch_url(url).await?;

    // Apply line range and character limits
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();

    let from = (start_line - 1).min(total_lines);
    let to = end_line
        .map(|e| e.min(total_lines))
        .unwrap_or_else(|| (from + MAX_OUTPUT_LINES).min(total_lines));

    if from > to {
        return Ok(json!(format!(
            "Error: start_line ({}) is greater than end_line ({}).",
            start_line,
            end_line.unwrap_or(0)
        )));
    }

    let slice = &lines[from..to];
    let mut result_text: String = slice.join("\n");

    let truncated_lines = to < total_lines;
    let truncated_chars = if result_text.len() > MAX_OUTPUT_CHARS {
        result_text = result_text.chars().take(MAX_OUTPUT_CHARS).collect();
        true
    } else {
        false
    };

    let mut notes = Vec::new();
    if truncated_lines {
        notes.push(format!(
            "Output truncated: showing lines {}-{} of {}. Use start_line/end_line to read more.",
            start_line, to, total_lines
        ));
    }
    if truncated_chars {
        notes.push(format!(
            "Output truncated at {} characters. Use start_line/end_line to read more.",
            MAX_OUTPUT_CHARS
        ));
    }

    Ok(json!({
        "content": result_text,
        "url": url,
        "total_lines": total_lines,
        "lines_shown": format!("{}-{}", start_line, to),
        "notes": notes
    }))
}

/// Search the web using the Brave LLM Context API.
///
/// This uses Brave's LLM-optimized endpoint that returns pre-extracted content
/// (text, tables, code) ready for LLM consumption — no scraping needed.
pub async fn brave_search(
    args: HashMap<String, Value>,
    _config: AppConfig,
    api_key: &str,
) -> anyhow::Result<Value> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'query' is required"))?;

    // Maximum number of search results to consider for context extraction (1–50)
    let count = args
        .get("count")
        .and_then(|v| v.as_u64())
        .map(|v| v.min(50))
        .unwrap_or(20);

    // Approximate maximum tokens in the returned context (1024–32768)
    let max_tokens = args
        .get("maximum_number_of_tokens")
        .and_then(|v| v.as_u64())
        .map(|v| v.clamp(1024, 32768))
        .unwrap_or(8192);

    // Maximum URLs in the response (1–50)
    let max_urls = args
        .get("maximum_number_of_urls")
        .and_then(|v| v.as_u64())
        .map(|v| v.clamp(1, 50))
        .unwrap_or(20);

    // Relevance threshold: "strict", "balanced", "lenient", or "disabled"
    let context_threshold_mode = args
        .get("context_threshold_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("balanced");

    // Freshness filter: "pd" (24h), "pw" (7d), "pm" (31d), "py" (365d), or date range
    let freshness = args.get("freshness").and_then(|v| v.as_str()).unwrap_or("");

    // Country code (2-char)
    let country = args.get("country").and_then(|v| v.as_str()).unwrap_or("");

    // Search language preference.
    // Brave API uses "jp" for Japanese, not the more common "ja".
    // Automatically map "ja" → "jp" so callers don't hit a 422 error.
    let search_lang = match args
        .get("search_lang")
        .and_then(|v| v.as_str())
        .unwrap_or("")
    {
        "ja" => "jp",
        other => other,
    };

    let result = call_brave_llm_context(
        BraveLlmContextParams {
            query,
            count,
            max_tokens,
            max_urls,
            context_threshold_mode,
            freshness,
            country,
            search_lang,
        },
        api_key,
    )
    .await?;

    Ok(result)
}

async fn fetch_url(url: &str) -> anyhow::Result<String> {
    let res = CLIENT
        .get(url)
        .header("User-Agent", "Mozilla/5.0 (compatible; llsc/0.1)")
        .send()
        .await?;

    let content_type = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("text/plain")
        .split(';')
        .next()
        .unwrap_or("text/plain")
        .trim()
        .to_lowercase();

    if content_type == "application/pdf" {
        let bytes = res.bytes().await?;
        return Ok(format!(
            "[PDF content: {} bytes. PDF text extraction not supported in this tool; download the file and use read_file_content instead.]",
            bytes.len()
        ));
    }

    let body = res.text().await?;

    if content_type.contains("html") {
        Ok(html_to_text(&body))
    } else {
        Ok(body)
    }
}

fn html_to_text(html: &str) -> String {
    let re_script = regex::Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap();
    let re_style = regex::Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap();
    let cleaned = re_script.replace_all(html, "");
    let cleaned = re_style.replace_all(&cleaned, "");
    html2text::from_read(cleaned.as_bytes(), 100).unwrap()
}

/// Parameters for the Brave LLM Context API.
struct BraveLlmContextParams<'a> {
    query: &'a str,
    count: u64,
    max_tokens: u64,
    max_urls: u64,
    context_threshold_mode: &'a str,
    freshness: &'a str,
    country: &'a str,
    search_lang: &'a str,
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
    if !params.country.is_empty() {
        body.insert("country".to_string(), json!(params.country));
    }
    if !params.search_lang.is_empty() {
        body.insert("search_lang".to_string(), json!(params.search_lang));
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
        // Try to parse the error response as JSON first (for structured API errors),
        // then fall back to raw text if that fails.
        let error_body = match res.json::<Value>().await {
            Ok(v) => v.to_string(),
            Err(_) => {
                // If JSON parsing fails, try reading as text
                // This won't happen normally since we removed the manual Accept-Encoding header,
                // but serves as a safety net.
                "Unable to read error response body".to_string()
            }
        };
        return Err(anyhow::anyhow!(
            "Brave LLM Context API error ({}): {}",
            status,
            error_body
        ));
    }

    let data: Value = res.json().await?;

    // Build a compact, LLM-friendly output
    let mut grounding_entries = Vec::new();

    // Process generic grounding data
    if let Some(generic) = data
        .get("grounding")
        .and_then(|g| g.get("generic"))
        .and_then(|g| g.as_array())
    {
        for entry in generic {
            let e_url = entry.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let e_title = entry.get("title").and_then(|v| v.as_str()).unwrap_or("");
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

    // Process POI data (if present, from local recall)
    let poi = data.get("grounding").and_then(|g| g.get("poi"));

    // Process map data (if present, from local recall)
    let map_results = data
        .get("grounding")
        .and_then(|g| g.get("map"))
        .and_then(|m| m.as_array())
        .cloned();

    // Extract source metadata
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

fn validate_url_ssrf(url: &str) -> anyhow::Result<()> {
    use std::net::IpAddr;
    let parsed = url::Url::parse(url).map_err(|e| anyhow::anyhow!("Invalid URL: {}", e))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(anyhow::anyhow!("Only http/https URLs are allowed"));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("URL has no host"))?;
    if let Ok(ip) = host.parse::<IpAddr>() {
        check_ip(ip)?;
    }
    let lower = host.to_lowercase();
    if lower == "localhost" || lower.ends_with(".local") || lower.ends_with(".internal") {
        return Err(anyhow::anyhow!("SSRF protection: blocked host '{}'", host));
    }
    Ok(())
}

fn check_ip(ip: std::net::IpAddr) -> anyhow::Result<()> {
    use std::net::IpAddr;
    let blocked = match ip {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || v4.is_documentation()
        }
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified() || v6.is_multicast(),
    };
    if blocked {
        return Err(anyhow::anyhow!(
            "SSRF protection: blocked IP address '{}'",
            ip
        ));
    }
    Ok(())
}
