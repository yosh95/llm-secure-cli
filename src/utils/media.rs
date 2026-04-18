use crate::llm::models::DataSource;
use base64::{engine::general_purpose, Engine as _};
use std::fs;
use std::path::Path;
use url::Url;

pub async fn fetch_url_content(
    url_str: &str,
    _pdf_as_base64: bool,
) -> anyhow::Result<(String, String)> {
    let _url = Url::parse(url_str)?;
    // SSRF validation omitted for brevity in this step, but recommended.

    let client = reqwest::Client::new();
    let res = client.get(url_str).send().await?;

    let content_type = res
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("text/plain")
        .split(';')
        .next()
        .unwrap_or("text/plain")
        .to_string();

    if content_type == "application/pdf" {
        let bytes = res.bytes().await?;
        let b64 = general_purpose::STANDARD.encode(bytes);
        Ok((b64, content_type))
    } else if content_type.contains("html") {
        let html = res.text().await?;
        let text = html_to_text(&html);
        Ok((text, "text/plain".to_string()))
    } else if content_type.starts_with("text/") || content_type.contains("json") {
        let text = res.text().await?;
        Ok((text, "text/plain".to_string()))
    } else {
        let bytes = res.bytes().await?;
        let b64 = general_purpose::STANDARD.encode(bytes);
        Ok((b64, content_type))
    }
}

/// Convert HTML to readable plain text, stripping scripts/styles first.
pub fn html_to_text(html: &str) -> String {
    use regex::Regex;
    // Strip <script> and <style> blocks
    let re_script = Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap();
    let re_style = Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap();
    let cleaned = re_script.replace_all(html, "");
    let cleaned = re_style.replace_all(&cleaned, "");
    html2text::from_read(cleaned.as_bytes(), 100)
}

pub fn process_file(path: &Path, _pdf_as_base64: bool) -> anyhow::Result<DataSource> {
    let _content_type = "text/plain"; // Simplified; use mime_guess in production
    let metadata = std::collections::HashMap::new();

    // In a real implementation, we'd use a crate like `infer` to detect mime type
    let bytes = fs::read(path)?;

    // Check if it's likely text
    if let Ok(text) = String::from_utf8(bytes.clone()) {
        Ok(DataSource {
            content: serde_json::Value::String(text),
            content_type: "text/plain".to_string(),
            is_file_or_url: true,
            metadata,
        })
    } else {
        let b64 = general_purpose::STANDARD.encode(bytes);
        Ok(DataSource {
            content: serde_json::Value::String(b64),
            content_type: "application/octet-stream".to_string(), // Placeholder
            is_file_or_url: true,
            metadata,
        })
    }
}

pub async fn process_single_source(source: &str, pdf_as_base64: bool) -> Option<DataSource> {
    if source.starts_with("http://") || source.starts_with("https://") {
        if let Ok((content, content_type)) = fetch_url_content(source, pdf_as_base64).await {
            return Some(DataSource {
                content: serde_json::Value::String(content),
                content_type,
                is_file_or_url: true,
                metadata: std::collections::HashMap::new(),
            });
        }
    } else {
        let path = Path::new(source);
        if path.exists() {
            if let Ok(ds) = process_file(path, pdf_as_base64) {
                return Some(ds);
            }
        }
    }
    None
}

pub async fn process_sources(sources: Vec<String>) -> Vec<DataSource> {
    let mut results = Vec::new();
    for s in sources {
        if s.starts_with("http://") || s.starts_with("https://") {
            if let Ok((content, content_type)) = fetch_url_content(&s, true).await {
                results.push(DataSource {
                    content: serde_json::Value::String(content),
                    content_type,
                    is_file_or_url: true,
                    metadata: std::collections::HashMap::new(),
                });
            }
        } else {
            let path = Path::new(&s);
            if path.exists() {
                if let Ok(ds) = process_file(path, true) {
                    results.push(ds);
                }
            } else {
                // Treat as raw text
                results.push(DataSource {
                    content: serde_json::Value::String(s),
                    content_type: "text/plain".to_string(),
                    is_file_or_url: false,
                    metadata: std::collections::HashMap::new(),
                });
            }
        }
    }
    results
}
