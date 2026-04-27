use crate::config::models::AppConfig;
use crate::consts::{MAX_OUTPUT_CHARS, MAX_OUTPUT_LINES};
use serde_json::{Value, json};
use std::collections::HashMap;

/// Fetch a web page URL or PDF URL and convert the content to Markdown/text.
/// For PDFs, text content will be extracted.
/// Respects MAX_OUTPUT_LINES and MAX_OUTPUT_CHARS limits.
pub fn read_url_content(args: HashMap<String, Value>, _config: AppConfig) -> anyhow::Result<Value> {
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

    // Build a blocking call
    let rt = tokio::runtime::Handle::try_current();
    let content = if rt.is_ok() {
        // Already inside an async context: use block_in_place
        tokio::task::block_in_place(|| fetch_url(url))?
    } else {
        fetch_url(url)?
    };

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

/// Search the web using the Brave Search API.
/// Returns titles, snippets, and URLs of relevant results.
pub fn brave_search(args: HashMap<String, Value>, _config: AppConfig) -> anyhow::Result<Value> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'query' is required"))?;

    let count = args
        .get("count")
        .and_then(|v| v.as_u64())
        .map(|v| v.min(20) as usize)
        .unwrap_or(10);

    let api_key = crate::config::CONFIG_MANAGER
        .get_api_key("brave")
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Brave Search API key not configured. Set BRAVE_API_KEY environment variable."
            )
        })?;

    let rt = tokio::runtime::Handle::try_current();
    let results = if rt.is_ok() {
        tokio::task::block_in_place(|| call_brave_api(query, count, &api_key))?
    } else {
        call_brave_api(query, count, &api_key)?
    };

    Ok(json!({
        "query": query,
        "count": results.len(),
        "results": results
    }))
}

// ---------------------------------------------------------------------------
// Internal async helpers
// ---------------------------------------------------------------------------

/// Fetch URL content and convert HTML to Markdown-like text.
fn fetch_url(url: &str) -> anyhow::Result<String> {
    let res = ureq::get(url)
        .header("User-Agent", "Mozilla/5.0 (compatible; llsc/0.1)")
        .call()?;

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
        // For PDFs, return a note
        let mut bytes = Vec::new();
        use std::io::Read;
        res.into_body().into_reader().read_to_end(&mut bytes)?;
        return Ok(format!(
            "[PDF content: {} bytes. PDF text extraction not supported in this tool; \
             download the file and use read_file_content instead.]",
            bytes.len()
        ));
    }

    let body = res.into_body().read_to_string()?;

    if content_type.contains("html") {
        Ok(html_to_text(&body))
    } else {
        Ok(body)
    }
}

/// Convert HTML to readable plain text using html2text crate.
fn html_to_text(html: &str) -> String {
    // Remove script and style blocks first
    let re_script = regex::Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap();
    let re_style = regex::Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap();
    let cleaned = re_script.replace_all(html, "");
    let cleaned = re_style.replace_all(&cleaned, "");

    // Use html2text for conversion
    html2text::from_read(cleaned.as_bytes(), 100).unwrap()
}

/// Call Brave Search API and return a JSON array of results.
fn call_brave_api(query: &str, count: usize, api_key: &str) -> anyhow::Result<Vec<Value>> {
    let url = format!(
        "https://api.search.brave.com/res/v1/web/search?q={}&count={}",
        urlencoding::encode(query),
        count
    );

    let res = ureq::get(&url)
        .header("Accept", "application/json")
        .header("X-Subscription-Token", api_key)
        .call()?;

    let data: serde_json::Value = res.into_body().read_json()?;

    let mut results = Vec::new();
    if let Some(web) = data
        .get("web")
        .and_then(|w| w.get("results"))
        .and_then(|r| r.as_array())
    {
        for item in web.iter().take(count) {
            results.push(json!({
                "title": item.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                "url": item.get("url").and_then(|v| v.as_str()).unwrap_or(""),
                "description": item.get("description").and_then(|v| v.as_str()).unwrap_or(""),
                "age": item.get("age").and_then(|v| v.as_str()).unwrap_or(""),
            }));
        }
    }

    Ok(results)
}

/// Basic SSRF protection: reject private/loopback/reserved IP ranges.
///
/// Limitation: DNS-based SSRF (DNS rebinding) is not fully mitigated here.
/// When the host is a hostname (not a raw IP), only a short blocklist of
/// well-known loopback names (`localhost`, `*.local`, `*.internal`) is checked.
/// A hostname that resolves to a private IP at runtime (e.g. via split-horizon
/// DNS) will pass this check. For high-security environments, route all
/// outbound requests through a dedicated egress proxy with full DNS resolution
/// and IP filtering instead of relying solely on this check.
fn validate_url_ssrf(url: &str) -> anyhow::Result<()> {
    use std::net::IpAddr;

    let parsed = url::Url::parse(url).map_err(|e| anyhow::anyhow!("Invalid URL: {}", e))?;

    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(anyhow::anyhow!("Only http/https URLs are allowed"));
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("URL has no host"))?;

    // If host is an IP address, validate it directly
    if let Ok(ip) = host.parse::<IpAddr>() {
        check_ip(ip)?;
    }
    // If it's a hostname, we trust the OS resolver; a full DNS-based check
    // would require an async resolver here. For now, block obvious loopback names.
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
