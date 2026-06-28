use crate::llm::models::DataSource;
use base64::{Engine as _, engine::general_purpose};
use dirs;
use jiff::Zoned;
use std::fs;
use std::path::Path;

/// Process a single source (file path or URL) and return a DataSource.
/// This is called from chat.rs for CLI sources (not from /attach anymore).
pub fn process_single_source(source: &str, _pdf_as_base64: bool) -> Option<DataSource> {
    if source.starts_with("http://") || source.starts_with("https://") {
        tracing::info!("Fetching URL: {source}");
        if let Ok((content, content_type)) = fetch_url_content(source) {
            let size_kb = content.len() as f64 / 1024.0;
            tracing::info!("Fetched URL ({size_kb:.1} KiB): {source}");
            return Some(DataSource {
                content: serde_json::Value::String(content),
                content_type,
                is_file_or_url: true,
                metadata: std::collections::HashMap::new(),
            });
        }
        tracing::warn!("Failed to fetch URL: {source}");
    } else {
        let path = Path::new(source);
        if path.exists() {
            tracing::info!("Reading file: {}", source);
            if let Ok(ds) = process_file(path) {
                let size_kb = match ds.content.as_str() {
                    Some(s) => s.len() as f64 / 1024.0,
                    None => 0.0,
                };
                tracing::info!("Read file ({size_kb:.1} KiB): {}", source);
                return Some(ds);
            }
            tracing::warn!("Failed to read file: {}", source);
        }
    }
    None
}

pub fn process_sources(sources: Vec<String>, _pdf_as_base64: bool) -> Vec<DataSource> {
    let mut results = Vec::new();
    for s in sources {
        if s.starts_with("http://") || s.starts_with("https://") {
            tracing::info!("Fetching URL: {s}");
            if let Ok((content, content_type)) = fetch_url_content(&s) {
                let size_kb = content.len() as f64 / 1024.0;
                tracing::info!("Fetched URL ({size_kb:.1} KiB): {s}");
                results.push(DataSource {
                    content: serde_json::Value::String(content),
                    content_type,
                    is_file_or_url: true,
                    metadata: std::collections::HashMap::new(),
                });
            } else {
                tracing::warn!("Failed to fetch URL: {s}");
            }
        } else {
            let path = Path::new(&s);
            if path.exists() {
                tracing::info!("Reading file: {s}");
                if let Ok(ds) = process_file(path) {
                    let size_kb = match ds.content.as_str() {
                        Some(c) => c.len() as f64 / 1024.0,
                        None => 0.0,
                    };
                    tracing::info!("Read file ({size_kb:.1} KiB): {s}");
                    results.push(ds);
                } else {
                    tracing::warn!("Failed to read file: {s}");
                }
            } else {
                // Treat as raw text
                tracing::info!(
                    "Using inline text ({len} chars): {truncated}",
                    len = s.len(),
                    truncated = if s.len() > 80 {
                        let mut t = s[..77].to_string();
                        t.push_str("...");
                        t
                    } else {
                        s.clone()
                    },
                );
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

pub fn process_file(path: &Path) -> anyhow::Result<DataSource> {
    let mut metadata = std::collections::HashMap::new();
    if let Some(filename) = path.file_name().and_then(|s| s.to_str()) {
        metadata.insert("filename".to_string(), serde_json::json!(filename));
    }
    let bytes = fs::read(path)?;

    // If it's valid UTF-8 text, treat as text/plain
    if let Ok(text) = String::from_utf8(bytes.clone()) {
        return Ok(DataSource {
            content: serde_json::Value::String(text),
            content_type: "text/plain".to_string(),
            is_file_or_url: true,
            metadata,
        });
    }

    // Binary file: send as base64
    let b64 = general_purpose::STANDARD.encode(bytes);
    Ok(DataSource {
        content: serde_json::Value::String(b64),
        content_type: "application/octet-stream".to_string(),
        is_file_or_url: true,
        metadata,
    })
}

fn fetch_url_content(url_str: &str) -> anyhow::Result<(String, String)> {
    let url_str = url_str.to_string();

    let (content_type, bytes) = crate::core::session::run_cancellable(move || {
        let response = crate::utils::http::CLIENT
            .get(&url_str)
            .call()
            .map_err(|e| anyhow::anyhow!("Failed to fetch URL: {e}"))?;

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|ct| ct.split(';').next().unwrap_or("text/plain").to_string())
            .unwrap_or_else(|| "text/plain".to_string());

        let bytes = response
            .into_body()
            .read_to_vec()
            .map_err(|e| anyhow::anyhow!("Failed to read response body: {e}"))?;

        Ok::<_, anyhow::Error>((content_type, bytes))
    })?;

    if content_type.starts_with("text/") || content_type.contains("json") {
        let text = String::from_utf8_lossy(&bytes).to_string();
        Ok((text, "text/plain".to_string()))
    } else {
        let b64 = general_purpose::STANDARD.encode(bytes);
        Ok((b64, content_type))
    }
}

pub fn save_media(b64_data: &str, mime_type: &str, save_path: &str) -> anyhow::Result<String> {
    let bytes = general_purpose::STANDARD.decode(b64_data)?;
    let extension = match mime_type {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/mpeg" => "mpeg",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/wav" => "wav",
        "audio/ogg" => "ogg",
        "audio/pcm" => "pcm",
        _ => "bin",
    };

    let timestamp = Zoned::now().strftime("%Y%m%d_%H%M%S");
    let filename = format!("generated_{timestamp}.{extension}");

    let mut path = Path::new(save_path).to_path_buf();
    // Expand ~ if present
    if path.starts_with("~")
        && let Some(home) = dirs::home_dir()
        && let Ok(stripped) = path.strip_prefix("~")
    {
        path = home.join(stripped);
    }

    fs::create_dir_all(&path)?;
    let full_path = path.join(filename);
    fs::write(&full_path, bytes)?;

    Ok(full_path.to_string_lossy().to_string())
}

/// Find the most recently modified file in the given directory.
/// Returns `None` if the directory does not exist or contains no files.
#[must_use]
pub fn find_latest_media(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let dir = if dir.starts_with("~") {
        let home = dirs::home_dir()?;
        let stripped = dir.strip_prefix("~").ok()?;
        home.join(stripped)
    } else {
        dir.to_path_buf()
    };

    let entries = std::fs::read_dir(&dir).ok()?;
    entries
        .filter_map(std::result::Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .max_by(|a, b| {
            let a_mtime = std::fs::metadata(a).ok().and_then(|m| m.modified().ok());
            let b_mtime = std::fs::metadata(b).ok().and_then(|m| m.modified().ok());
            a_mtime.cmp(&b_mtime)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_file_text() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "Hello, world!")?;
        let ds = process_file(&path)?;
        // as_str() returns Option<&str>; assert_eq with Some avoids unwrap
        assert_eq!(ds.content.as_str(), Some("Hello, world!"));
        assert_eq!(ds.content_type, "text/plain");
        Ok(())
    }

    #[test]
    fn test_process_file_binary() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("test.bin");
        let bytes = vec![0u8, 1, 2, 3, 255];
        std::fs::write(&path, &bytes)?;
        let ds = process_file(&path)?;
        assert_eq!(ds.content_type, "application/octet-stream");
        // Use match to safely extract the string without unwrap/expect
        let text = match ds.content.as_str() {
            Some(s) => s,
            None => return Err(anyhow::anyhow!("content should be a string")),
        };
        let decoded = general_purpose::STANDARD.decode(text)?;
        assert_eq!(decoded, bytes);
        Ok(())
    }
}
