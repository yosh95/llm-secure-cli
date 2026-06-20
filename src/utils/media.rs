use crate::llm::models::DataSource;
use crate::utils::http::CLIENT;
use base64::{Engine as _, engine::general_purpose};
use chrono;
use dirs;
use mime_guess;
use std::fs;
use std::path::Path;

pub fn fetch_url_content(url_str: &str, pdf_as_base64: bool) -> anyhow::Result<(String, String)> {
    let url_str = url_str.to_string();

    let (content_type, bytes) = crate::core::session::run_cancellable(move || {
        let response = CLIENT
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

    if content_type == "application/pdf" {
        if pdf_as_base64 {
            let b64 = general_purpose::STANDARD.encode(bytes);
            Ok((b64, content_type))
        } else {
            let text = pdf_extract::extract_text_from_mem(&bytes)?;
            Ok((text, "text/plain".to_string()))
        }
    } else if content_type.contains("html") {
        let html = String::from_utf8_lossy(&bytes);
        let text = html_to_text(&html)?;
        Ok((text, "text/plain".to_string()))
    } else if content_type.starts_with("text/") || content_type.contains("json") {
        let text = String::from_utf8_lossy(&bytes).to_string();
        Ok((text, "text/plain".to_string()))
    } else {
        let b64 = general_purpose::STANDARD.encode(bytes);
        Ok((b64, content_type))
    }
}

/// Convert HTML to Markdown using html-to-markdown-rs.
/// Strips common navigation/header/footer elements to produce cleaner content for LLM consumption.
pub fn html_to_text(html: &str) -> anyhow::Result<String> {
    use html_to_markdown_rs::{
        ConversionOptions, PreprocessingOptions, PreprocessingPreset, convert,
    };

    let options = ConversionOptions::builder()
        // Exclude HTML elements that are typically navigation or boilerplate,
        // not useful for LLM content extraction:
        // - <head>: HTML metadata, stylesheets, scripts (should not appear in body anyway)
        // - <header>: Page header/banner
        // - <nav>: Navigation menus
        // - <footer>: Page footer
        // - [role="banner"]: ARIA banner landmark
        // - [role="navigation"]: ARIA navigation landmark
        // - [role="contentinfo"]: ARIA contentinfo landmark (typically footer info)
        .exclude_selectors(vec![
            "head".into(),
            "header".into(),
            "nav".into(),
            "footer".into(),
            "[role='banner']".into(),
            "[role='navigation']".into(),
            "[role='contentinfo']".into(),
            // Exclude inline SVG elements: LLMs cannot interpret SVG markup,
            // and they waste tokens (can be tens of KB per icon/logo).
            "svg".into(),
            // Exclude <img> tags with data URIs (e.g. data:image/svg+xml;base64,...).
            // These embed the image bytes directly and are useless for LLMs.
            // Normal image paths (e.g. /img/photo.png) are kept so alt-text and
            // file context remain available.
            "img[src^='data:']".into(),
        ])
        .default_title(false)
        .extract_metadata(false)
        .preprocessing(PreprocessingOptions {
            enabled: true,
            preset: PreprocessingPreset::Standard,
            remove_navigation: true,
            remove_forms: false,
        })
        .build();

    let result = convert(html, Some(options))?;
    let raw = match result.content {
        Some(content) => content,
        None => return Ok(String::new()),
    };

    // Collapse 3+ consecutive blank lines → 1 blank line,
    // and strip leading/trailing blank lines.
    Ok(collapse_blank_lines(&raw))
}

/// Collapse 3 or more consecutive blank lines into a single blank line,
/// and trim leading/trailing empty lines.
fn collapse_blank_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank_count = 0u32;
    let mut first_non_blank = false;

    for line in text.lines() {
        if line.trim().is_empty() {
            blank_count += 1;
        } else {
            // Flush pending blanks: at most 1 blank line
            if first_non_blank && blank_count > 0 {
                out.push('\n');
            }
            if first_non_blank {
                out.push('\n');
            }
            out.push_str(line);
            first_non_blank = true;
            blank_count = 0;
        }
    }
    out
}

pub fn process_file(path: &Path, pdf_as_base64: bool) -> anyhow::Result<DataSource> {
    let mut metadata = std::collections::HashMap::new();
    if let Some(filename) = path.file_name().and_then(|s| s.to_str()) {
        metadata.insert("filename".to_string(), serde_json::json!(filename));
    }
    let bytes = fs::read(path)?;

    // Use mime_guess to determine the content type
    let mime_type = mime_guess::from_path(path)
        .first_raw()
        .unwrap_or("application/octet-stream");

    if mime_type == "application/pdf" && !pdf_as_base64 {
        let text = pdf_extract::extract_text_from_mem(&bytes)?;
        return Ok(DataSource {
            content: serde_json::Value::String(text),
            content_type: "text/plain".to_string(),
            is_file_or_url: true,
            metadata,
        });
    }

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

pub fn process_single_source(source: &str, pdf_as_base64: bool) -> Option<DataSource> {
    if source.starts_with("http://") || source.starts_with("https://") {
        if let Ok((content, content_type)) = fetch_url_content(source, pdf_as_base64) {
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

pub fn process_sources(sources: Vec<String>, pdf_as_base64: bool) -> Vec<DataSource> {
    let mut results = Vec::new();
    for s in sources {
        if s.starts_with("http://") || s.starts_with("https://") {
            if let Ok((content, content_type)) = fetch_url_content(&s, pdf_as_base64) {
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
                if let Ok(ds) = process_file(path, pdf_as_base64) {
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

    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
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

/// Open a file using the system's default application.
///
/// Uses the `open` crate, which delegates to:
/// - `xdg-open` on Linux
/// - `open` on macOS
/// - `start` on Windows
///
/// Returns an error with a platform-not-supported message if opening fails.
pub fn open_file_with_default_app(path: &std::path::Path) -> anyhow::Result<()> {
    let path_str = path.to_string_lossy();
    open::that(path)
        .map_err(|e| anyhow::anyhow!(
            "Failed to open file: {e}\n             Your platform may not support opening files via command line.\n             You can manually open the file at: {path_str}"
        ))
}
