use crate::config::models::AppConfig;
use chrono::DateTime;
use ignore::WalkBuilder;
use ignore::overrides::OverrideBuilder;
use regex::Regex;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::sync::Arc;

use crate::consts::{MAX_OUTPUT_CHARS, MAX_OUTPUT_LINES};

const MAX_FILE_READ_SIZE: u64 = 5 * 1024 * 1024; // 5MB
const SEARCH_TIMEOUT_SECS: u64 = 55;
const MAX_SEARCH_RESULTS: usize = 300;

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

pub fn list_files_in_directory(
    args: HashMap<String, Value>,
    config: Arc<AppConfig>,
) -> anyhow::Result<Value> {
    let directory = args
        .get("directory")
        .and_then(|v| v.as_str())
        .unwrap_or(".");
    let depth = args.get("depth").and_then(|v| v.as_i64()).unwrap_or(1) as usize;
    let include_hidden = args
        .get("include_hidden")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let max_files = args
        .get("max_files")
        .and_then(|v| v.as_i64())
        .unwrap_or(500) as usize;
    let ignore_patterns: Vec<String> = args
        .get("ignore_patterns")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let base_path =
        match crate::security::path_validator::validate_path(directory, &config.security) {
            Ok(p) => p,
            Err(e) => return Err(anyhow::anyhow!("Security Error: {}", e)),
        };

    if !base_path.exists() {
        return Ok(json!(format!(
            "Error: Directory '{}' does not exist.",
            directory
        )));
    }
    if !base_path.is_dir() {
        return Ok(json!(format!("Error: '{}' is not a directory.", directory)));
    }

    let mut results = Vec::new();
    let mut file_count = 0usize;
    let mut ignored_count = 0usize;

    let mut override_builder = OverrideBuilder::new(&base_path);
    for pattern in ignore_patterns {
        let _ = override_builder.add(&format!("!{}", pattern));
    }
    let overrides = override_builder
        .build()
        .unwrap_or(ignore::overrides::Override::empty());

    let mut walker = WalkBuilder::new(&base_path);
    walker
        .max_depth(Some(depth))
        .hidden(!include_hidden)
        .git_ignore(true)
        .require_git(false)
        .overrides(overrides);

    for result in walker.build() {
        match result {
            Ok(entry) => {
                if entry.path() == base_path {
                    continue;
                }

                if file_count >= max_files {
                    break;
                }

                let path = entry.path();
                let rel_path = path
                    .strip_prefix(&base_path)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .replace('\\', "/");

                if let Ok(metadata) = entry.metadata() {
                    let mtime = metadata
                        .modified()
                        .ok()
                        .map(|t| {
                            let dt: DateTime<chrono::Utc> = t.into();
                            dt.format("%Y-%m-%d %H:%M:%S").to_string()
                        })
                        .unwrap_or_else(|| "Unknown".to_string());

                    if metadata.is_dir() {
                        results.push(json!({
                            "type": "dir",
                            "path": rel_path,
                            "last_modified": mtime
                        }));
                    } else {
                        results.push(json!({
                            "type": "file",
                            "path": rel_path,
                            "size": metadata.len(),
                            "last_modified": mtime
                        }));
                    }
                    file_count += 1;
                }
            }
            Err(_) => {
                ignored_count += 1;
            }
        }
    }

    Ok(json!({
        "files": results,
        "truncated": file_count >= max_files,
        "ignored_count": ignored_count
    }))
}

pub fn read_file_content(
    args: HashMap<String, Value>,
    config: Arc<AppConfig>,
) -> anyhow::Result<Value> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'path' argument"))?;
    let start_line = args.get("start_line").and_then(|v| v.as_i64()).unwrap_or(1) as usize;
    let end_line = args
        .get("end_line")
        .and_then(|v| v.as_i64())
        .map(|n| n as usize);
    let with_line_numbers = args
        .get("with_line_numbers")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let path = match crate::security::path_validator::validate_path(path_str, &config.security) {
        Ok(p) => p,
        Err(e) => return Err(anyhow::anyhow!("Security Error: {}", e)),
    };
    if !path.is_file() {
        return Ok(json!(format!("Error: '{}' is not a file.", path_str)));
    }

    let metadata = fs::metadata(&path)?;
    if metadata.len() > MAX_FILE_READ_SIZE {
        return Ok(json!(format!(
            "Error: File '{}' is too large ({}) to read directly. Max is 5MB.",
            path_str,
            format_size(metadata.len())
        )));
    }

    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => {
            return Ok(json!(format!(
                "Error: '{}' appears to be a binary file.",
                path_str
            )));
        }
    };

    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();
    let start = (start_line.saturating_sub(1)).min(total_lines);
    let end = end_line.map(|e| e.min(total_lines)).unwrap_or(total_lines);

    if start > end {
        return Ok(json!(format!(
            "Error: start_line ({}) is greater than end_line ({}).",
            start_line,
            end_line.unwrap_or(0)
        )));
    }

    let selected = &lines[start..end];
    let is_truncated = selected.len() > MAX_OUTPUT_LINES;
    let limited: Vec<&str> = selected.iter().take(MAX_OUTPUT_LINES).cloned().collect();

    let mut output = if with_line_numbers {
        limited
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{:4} | {}", start + i + 1, line))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        limited.join("\n")
    };

    if is_truncated {
        let shown_end = start + limited.len();
        output = format!(
            "\nIMPORTANT: The file content has been truncated.\n\
             Status: Showing lines {}-{} of {} total lines.\n\
             Action: To read more of the file, use 'start_line' and 'end_line' in a subsequent call. \
             For example, use start_line: {}.\n\n\
             --- FILE CONTENT (truncated) ---\n{}",
            start + 1,
            shown_end,
            total_lines,
            shown_end + 1,
            output
        );
    }

    // Truncate by chars if needed
    if output.len() > MAX_OUTPUT_CHARS {
        let truncated: String = output.chars().take(MAX_OUTPUT_CHARS).collect();
        Ok(json!(truncated))
    } else {
        Ok(json!(output))
    }
}

pub fn read_many_files(
    args: HashMap<String, Value>,
    config: Arc<AppConfig>,
) -> anyhow::Result<Value> {
    let paths = args
        .get("paths")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("Missing 'paths' argument"))?;

    let mut results = Vec::new();

    for path_val in paths {
        let path_str = path_val.as_str().unwrap_or("");
        if path_str.is_empty() {
            continue;
        }

        let path = match crate::security::path_validator::validate_path(path_str, &config.security)
        {
            Ok(p) => p,
            Err(e) => {
                results.push(json!({
                    "path": path_str,
                    "error": format!("Security Error: {}", e)
                }));
                continue;
            }
        };

        if !path.is_file() {
            results.push(json!({
                "path": path_str,
                "error": "Not a file or does not exist."
            }));
            continue;
        }

        match fs::read_to_string(&path) {
            Ok(content) => {
                results.push(json!({
                    "path": path_str,
                    "content": content
                }));
            }
            Err(e) => {
                results.push(json!({
                    "path": path_str,
                    "error": format!("Read error: {}", e)
                }));
            }
        }
    }

    Ok(json!({ "results": results }))
}

pub fn grep_files(args: HashMap<String, Value>, config: Arc<AppConfig>) -> anyhow::Result<Value> {
    let directory = args
        .get("directory")
        .and_then(|v| v.as_str())
        .unwrap_or(".");
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'query' argument"))?;
    let file_pattern = args.get("file_pattern").and_then(|v| v.as_str());

    let base_path =
        match crate::security::path_validator::validate_path(directory, &config.security) {
            Ok(p) => p,
            Err(e) => return Err(anyhow::anyhow!("Security Error: {}", e)),
        };

    let regex = match Regex::new(query) {
        Ok(r) => r,
        Err(e) => return Ok(json!(format!("Error: Invalid regex pattern: {}", e))),
    };

    let mut results = Vec::new();
    let start_time = std::time::Instant::now();

    let mut walker = WalkBuilder::new(&base_path);
    walker.git_ignore(true).require_git(false).hidden(true);

    if let Some(pattern) = file_pattern {
        let mut override_builder = OverrideBuilder::new(&base_path);
        let _ = override_builder.add(pattern);
        if let Ok(overrides) = override_builder.build() {
            walker.overrides(overrides);
        }
    }

    for result in walker.build() {
        if start_time.elapsed().as_secs() > SEARCH_TIMEOUT_SECS {
            break;
        }

        if let Ok(entry) = result
            && entry.file_type().map(|ft| ft.is_file()).unwrap_or(false)
        {
            let path = entry.path();

            // Skip large files
            if let Ok(meta) = path.metadata()
                && meta.len() > MAX_FILE_READ_SIZE
            {
                continue;
            }

            if let Ok(content) = fs::read_to_string(path) {
                for (line_no, line) in content.lines().enumerate() {
                    if regex.is_match(line) {
                        let rel = path.strip_prefix(&base_path).unwrap_or(path);
                        results.push(json!({
                            "file": rel.to_string_lossy().replace('\\', "/"),
                            "line": line_no + 1,
                            "text": line.trim().to_string()
                        }));
                        if results.len() >= MAX_SEARCH_RESULTS {
                            break;
                        }
                    }
                }
            }
        }
        if results.len() >= MAX_SEARCH_RESULTS {
            break;
        }
    }

    Ok(json!({
        "matches": results,
        "truncated": results.len() >= MAX_SEARCH_RESULTS || start_time.elapsed().as_secs() > SEARCH_TIMEOUT_SECS,
    }))
}

pub fn search_files(args: HashMap<String, Value>, config: Arc<AppConfig>) -> anyhow::Result<Value> {
    let directory = args
        .get("directory")
        .and_then(|v| v.as_str())
        .unwrap_or(".");
    let pattern = args
        .get("pattern")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'pattern' argument"))?;
    let exclude_patterns: Vec<String> = args
        .get("exclude_patterns")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let base_path =
        match crate::security::path_validator::validate_path(directory, &config.security) {
            Ok(p) => p,
            Err(e) => return Err(anyhow::anyhow!("Security Error: {}", e)),
        };

    let mut results = Vec::new();

    let mut override_builder = OverrideBuilder::new(&base_path);
    let _ = override_builder.add(pattern);
    for excl in exclude_patterns {
        let _ = override_builder.add(&format!("!{}", excl));
    }
    let overrides = override_builder
        .build()
        .unwrap_or(ignore::overrides::Override::empty());

    let mut walker = WalkBuilder::new(&base_path);
    walker
        .git_ignore(true)
        .require_git(false)
        .hidden(true)
        .overrides(overrides);

    for result in walker.build() {
        if let Ok(entry) = result {
            if entry.path() == base_path {
                continue;
            }
            let path = entry.path();
            let rel = path.strip_prefix(&base_path).unwrap_or(path);
            results.push(json!({
                "type": if path.is_dir() { "dir" } else { "file" },
                "path": rel.to_string_lossy().replace('\\', "/")
            }));
        }
        if results.len() >= MAX_SEARCH_RESULTS {
            break;
        }
    }

    Ok(json!({
        "results": results,
        "truncated": results.len() >= MAX_SEARCH_RESULTS
    }))
}
