use chrono::DateTime;
use regex::Regex;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::consts::{MAX_OUTPUT_CHARS, MAX_OUTPUT_LINES};

const MAX_FILE_READ_SIZE: u64 = 5 * 1024 * 1024; // 5MB
const SEARCH_TIMEOUT_SECS: u64 = 55;
const MAX_SEARCH_RESULTS: usize = 300;

const DEFAULT_EXCLUDE_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "cache",
    ".cache",
    "__pycache__",
    "venv",
    ".venv",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    "dist",
    "build",
    ".tox",
    ".idea",
    ".vscode",
    ".DS_Store",
    "target", // Rust build output
];

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

fn should_ignore(name: &str, include_hidden: bool, ignore_patterns: &[String]) -> bool {
    if !include_hidden && name.starts_with('.') {
        return true;
    }
    for pattern in ignore_patterns {
        if glob_match(pattern, name) {
            return true;
        }
    }
    DEFAULT_EXCLUDE_DIRS.contains(&name)
}

fn glob_match(pattern: &str, name: &str) -> bool {
    // Simple glob matching: * matches any sequence
    if pattern == name {
        return true;
    }
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return name.starts_with(prefix);
    }
    if let Some(suffix) = pattern.strip_prefix('*') {
        return name.ends_with(suffix);
    }
    false
}

pub fn list_files_in_directory(args: HashMap<String, Value>) -> anyhow::Result<Value> {
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

    let base_path = PathBuf::from(directory);
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
    results.push(format!(
        "{:<7} {:<20} {:>10}  {}",
        "[Type]", "[Last Modified (UTC)]", "[Size]", "[Full Path]"
    ));

    let mut file_count = 0usize;

    #[allow(clippy::too_many_arguments)]
    fn walk(
        current_path: &Path,
        base_path: &Path,
        current_depth: usize,
        max_depth: usize,
        include_hidden: bool,
        ignore_patterns: &[String],
        results: &mut Vec<String>,
        file_count: &mut usize,
        max_files: usize,
    ) {
        if current_depth > max_depth {
            return;
        }

        let mut entries: Vec<_> = match fs::read_dir(current_path) {
            Ok(rd) => rd.flatten().collect(),
            Err(_) => {
                results.push(format!(
                    "{:<7} {:<20} {:>10}  [DENIED] Permission Denied",
                    "[ERR]", "", ""
                ));
                return;
            }
        };

        // Sort: dirs first, then by name
        entries.sort_by(|a, b| {
            let a_is_dir = a.path().is_dir();
            let b_is_dir = b.path().is_dir();
            match (a_is_dir, b_is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.file_name().cmp(&b.file_name()),
            }
        });

        for entry in entries {
            if *file_count >= max_files {
                if *file_count == max_files {
                    results.push("\n... (Too many files, listing truncated)".to_string());
                    *file_count += 1;
                }
                return;
            }

            let name = entry.file_name().to_string_lossy().to_string();
            if should_ignore(&name, include_hidden, ignore_patterns) {
                continue;
            }

            let path = entry.path();
            let rel_path = path
                .strip_prefix(base_path)
                .unwrap_or(&path)
                .to_string_lossy();

            if let Ok(metadata) = path.metadata() {
                let mtime = metadata
                    .modified()
                    .ok()
                    .map(|t| {
                        let dt: DateTime<chrono::Utc> = t.into();
                        dt.format("%Y-%m-%d %H:%M:%S").to_string()
                    })
                    .unwrap_or_else(|| "Unknown".to_string());

                if path.is_dir() {
                    results.push(format!(
                        "{:<7} {:<20} {:>10}  {}/",
                        "[D]", mtime, "-", rel_path
                    ));
                    *file_count += 1;
                    walk(
                        &path,
                        base_path,
                        current_depth + 1,
                        max_depth,
                        include_hidden,
                        ignore_patterns,
                        results,
                        file_count,
                        max_files,
                    );
                } else {
                    let size = format_size(metadata.len());
                    results.push(format!(
                        "{:<7} {:<20} {:>10}  {}",
                        "[F]", mtime, size, rel_path
                    ));
                    *file_count += 1;
                }
            }
        }
    }

    walk(
        &base_path,
        &base_path,
        1,
        depth,
        include_hidden,
        &ignore_patterns,
        &mut results,
        &mut file_count,
        max_files,
    );

    if results.len() <= 1 {
        Ok(json!("No files found."))
    } else {
        Ok(json!(results.join("\n")))
    }
}

pub fn read_file_content(args: HashMap<String, Value>) -> anyhow::Result<Value> {
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

    let path = PathBuf::from(path_str);
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
            // Try as binary → report as binary
            return Ok(json!(format!(
                "Error: '{}' appears to be a binary file.",
                path_str
            )));
        }
    };

    let lines: Vec<&str> = content.lines().collect();
    let start = (start_line.saturating_sub(1)).min(lines.len());
    let end = end_line.map(|e| e.min(lines.len())).unwrap_or(lines.len());
    let selected = &lines[start..end];

    // Enforce output limits
    let limited: Vec<&str> = selected.iter().take(MAX_OUTPUT_LINES).cloned().collect();

    let output = if with_line_numbers {
        limited
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{:4} | {}", start + i + 1, line))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        limited.join("\n")
    };

    // Truncate by chars if needed
    if output.len() > MAX_OUTPUT_CHARS {
        Ok(json!(&output[..MAX_OUTPUT_CHARS]))
    } else {
        Ok(json!(output))
    }
}

pub fn grep_files(args: HashMap<String, Value>) -> anyhow::Result<Value> {
    let directory = args
        .get("directory")
        .and_then(|v| v.as_str())
        .unwrap_or(".");
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'query' argument"))?;
    let file_pattern = args.get("file_pattern").and_then(|v| v.as_str());

    let base_path = PathBuf::from(directory);
    if !base_path.exists() {
        return Ok(json!(format!(
            "Error: Directory '{}' does not exist.",
            directory
        )));
    }
    if !base_path.is_dir() {
        return Ok(json!(format!("Error: '{}' is not a directory.", directory)));
    }

    let regex = match Regex::new(query) {
        Ok(r) => r,
        Err(e) => return Ok(json!(format!("Error: Invalid regex pattern: {}", e))),
    };

    let mut results = Vec::new();
    let start_time = std::time::Instant::now();

    fn walk_grep(
        dir: &Path,
        base: &Path,
        regex: &Regex,
        file_pattern: Option<&str>,
        results: &mut Vec<String>,
        start_time: std::time::Instant,
        timeout: u64,
    ) -> bool {
        if start_time.elapsed().as_secs() > timeout {
            return false; // timed out
        }

        let entries = match fs::read_dir(dir) {
            Ok(rd) => rd.flatten().collect::<Vec<_>>(),
            Err(_) => return true,
        };

        for entry in entries {
            if start_time.elapsed().as_secs() > timeout {
                return false;
            }

            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();

            if name.starts_with('.') || DEFAULT_EXCLUDE_DIRS.contains(&name.as_str()) {
                continue;
            }

            if path.is_dir() {
                if !walk_grep(
                    &path,
                    base,
                    regex,
                    file_pattern,
                    results,
                    start_time,
                    timeout,
                ) {
                    return false;
                }
            } else if path.is_file() {
                // Check file pattern
                if let Some(pattern) = file_pattern {
                    if !glob_match(pattern, &name) {
                        continue;
                    }
                }

                // Skip large files
                if let Ok(meta) = path.metadata() {
                    if meta.len() > MAX_FILE_READ_SIZE {
                        continue;
                    }
                }

                // Skip binary files
                if let Ok(mut f) = fs::File::open(&path) {
                    use std::io::Read;
                    let mut buf = [0u8; 1024];
                    if let Ok(n) = f.read(&mut buf) {
                        if buf[..n].contains(&0u8) {
                            continue; // binary
                        }
                    }
                }

                if let Ok(content) = fs::read_to_string(&path) {
                    for (line_no, line) in content.lines().enumerate() {
                        if regex.is_match(line) {
                            let rel = path.strip_prefix(base).unwrap_or(&path);
                            results.push(format!(
                                "{}:{}:{}",
                                rel.display(),
                                line_no + 1,
                                line.trim()
                            ));
                            if results.len() >= MAX_SEARCH_RESULTS {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        true
    }

    let timed_out = !walk_grep(
        &base_path,
        &base_path,
        &regex,
        file_pattern,
        &mut results,
        start_time,
        SEARCH_TIMEOUT_SECS,
    );

    if timed_out {
        results.push(format!(
            "Error: Search timed out after {} seconds.",
            SEARCH_TIMEOUT_SECS
        ));
    }

    if results.is_empty() {
        Ok(json!("No matches found."))
    } else {
        let truncated = results.len() >= MAX_SEARCH_RESULTS;
        let mut output = results.join("\n");
        if truncated {
            output.push_str(&format!(
                "\n\n... (Total {} matches, truncated)",
                results.len()
            ));
        }
        Ok(json!(output))
    }
}

pub fn search_files(args: HashMap<String, Value>) -> anyhow::Result<Value> {
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

    let base_path = PathBuf::from(directory);
    if !base_path.exists() {
        return Ok(json!(format!(
            "Error: Directory '{}' does not exist.",
            directory
        )));
    }

    let mut results = Vec::new();

    fn walk_search(
        dir: &Path,
        base: &Path,
        pattern: &str,
        exclude_patterns: &[String],
        results: &mut Vec<String>,
    ) {
        let entries = match fs::read_dir(dir) {
            Ok(rd) => rd.flatten().collect::<Vec<_>>(),
            Err(_) => return,
        };

        let mut sorted_entries = entries;
        sorted_entries.sort_by_key(|e| e.file_name());

        for entry in sorted_entries {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();

            if name.starts_with('.') || DEFAULT_EXCLUDE_DIRS.contains(&name.as_str()) {
                continue;
            }

            if exclude_patterns.iter().any(|p| glob_match(p, &name)) {
                continue;
            }

            if glob_match(pattern, &name) {
                let rel = path.strip_prefix(base).unwrap_or(&path);
                let item_type = if path.is_dir() { "[D]" } else { "[F]" };
                results.push(format!("{} {}", item_type, rel.display()));
            }

            if path.is_dir() {
                walk_search(&path, base, pattern, exclude_patterns, results);
            }

            if results.len() >= MAX_SEARCH_RESULTS {
                return;
            }
        }
    }

    walk_search(
        &base_path,
        &base_path,
        pattern,
        &exclude_patterns,
        &mut results,
    );

    if results.is_empty() {
        Ok(json!("No files found matching the pattern."))
    } else {
        let truncated = results.len() >= MAX_SEARCH_RESULTS;
        let mut output = results.join("\n");
        if truncated {
            output.push_str("\n... (Listing truncated)");
        }
        Ok(json!(output))
    }
}
