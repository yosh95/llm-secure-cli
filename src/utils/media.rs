use crate::llm::models::DataSource;
use base64::{Engine as _, engine::general_purpose};
use chrono;
use dirs;
use mime_guess;
use std::fs;
use std::io::Read;
use std::path::Path;

pub async fn fetch_url_content(
    url_str: &str,
    _pdf_as_base64: bool,
) -> anyhow::Result<(String, String)> {
    let url = url_str.to_string();

    let res_result = tokio::task::spawn_blocking(move || ureq::get(&url).call()).await?;

    let res = match res_result {
        Ok(r) => r,
        Err(e) => return Err(anyhow::anyhow!("Failed to fetch URL: {}", e)),
    };

    let content_type = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("text/plain")
        .split(';')
        .next()
        .unwrap_or("text/plain")
        .to_string();

    if content_type == "application/pdf" {
        let mut bytes = Vec::new();
        res.into_body().into_reader().read_to_end(&mut bytes)?;
        let b64 = general_purpose::STANDARD.encode(bytes);
        Ok((b64, content_type))
    } else if content_type.contains("html") {
        let html = res.into_body().read_to_string()?;
        let text = html_to_text(&html);
        Ok((text, "text/plain".to_string()))
    } else if content_type.starts_with("text/") || content_type.contains("json") {
        let text = res.into_body().read_to_string()?;
        Ok((text, "text/plain".to_string()))
    } else {
        let mut bytes = Vec::new();
        res.into_body().into_reader().read_to_end(&mut bytes)?;
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
    html2text::from_read(cleaned.as_bytes(), 100).unwrap_or_else(|_| cleaned.to_string())
}

pub fn process_file(path: &Path, _pdf_as_base64: bool) -> anyhow::Result<DataSource> {
    let mut metadata = std::collections::HashMap::new();
    if let Some(filename) = path.file_name().and_then(|s| s.to_str()) {
        metadata.insert("filename".to_string(), serde_json::json!(filename));
    }
    let bytes = fs::read(path)?;

    // Use mime_guess to determine the content type
    let mime_type = mime_guess::from_path(path)
        .first_raw()
        .unwrap_or("application/octet-stream");

    // Check if it's likely text
    if let Ok(text) = String::from_utf8(bytes.clone()) {
        let content_type =
            if mime_type == "application/octet-stream" || mime_type.starts_with("text/") {
                "text/plain".to_string()
            } else {
                mime_type.to_string()
            };

        Ok(DataSource {
            content: serde_json::Value::String(text),
            content_type,
            is_file_or_url: true,
            metadata,
        })
    } else {
        let b64 = general_purpose::STANDARD.encode(bytes);
        Ok(DataSource {
            content: serde_json::Value::String(b64),
            content_type: mime_type.to_string(),
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
        if path.exists()
            && let Ok(ds) = process_file(path, pdf_as_base64)
        {
            return Some(ds);
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

pub fn save_image(b64_data: &str, mime_type: &str, save_path: &str) -> anyhow::Result<String> {
    let bytes = general_purpose::STANDARD.decode(b64_data)?;
    let extension = match mime_type {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "bin",
    };

    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let filename = format!("generated_{}.{}", timestamp, extension);

    let mut path = Path::new(save_path).to_path_buf();
    // Expand ~ if present
    if path.starts_with("~")
        && let Some(home) = dirs::home_dir()
    {
        path = home.join(path.strip_prefix("~").unwrap());
    }

    fs::create_dir_all(&path)?;
    let full_path = path.join(filename);
    fs::write(&full_path, bytes)?;

    Ok(full_path.to_string_lossy().to_string())
}
