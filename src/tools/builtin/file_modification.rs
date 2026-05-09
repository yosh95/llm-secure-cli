use crate::config::models::AppConfig;
use regex::Regex;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

/// Edit a file by replacing a specific block of text.
/// Tries multiple strategies: exact, flexible (indentation-aware), and regex (whitespace-insensitive).
pub fn edit_file(args: HashMap<String, Value>, config: Arc<AppConfig>) -> anyhow::Result<Value> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'path' is required"))?;

    let search_str = args
        .get("search")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'search' is required"))?;

    let replace_str = args
        .get("replace")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'replace' is required"))?;

    let allow_multiple = args
        .get("allow_multiple")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let dry_run = args
        .get("dry_run")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let path = match crate::security::path_validator::validate_path(path_str, &config.security) {
        Ok(p) => p,
        Err(e) => return Err(anyhow::anyhow!("Security Error: {}", e)),
    };
    if !path.exists() {
        return Err(anyhow::anyhow!("File not found: {}", path_str));
    }

    let original =
        fs::read_to_string(&path).map_err(|e| anyhow::anyhow!("Cannot read file: {}", e))?;

    if search_str.is_empty() {
        return Err(anyhow::anyhow!(
            "Search string cannot be empty. To create or overwrite a file, use create_or_overwrite_file tool or provide a non-empty search string."
        ));
    }

    // Strategy 1: Exact match
    if let Some((new_content, count)) =
        try_exact_edit(&original, search_str, replace_str, allow_multiple)
    {
        return finalize_edit(
            &path,
            &original,
            &new_content,
            dry_run,
            path_str,
            "exact",
            count,
        );
    }

    // Strategy 2: Flexible match (indentation-aware, line-by-line trim match)
    if let Some((new_content, count)) =
        try_flexible_edit(&original, search_str, replace_str, allow_multiple)
    {
        return finalize_edit(
            &path,
            &original,
            &new_content,
            dry_run,
            path_str,
            "flexible",
            count,
        );
    }

    // Strategy 3: Regex match (whitespace-insensitive)
    if let Some((new_content, count)) =
        try_regex_edit(&original, search_str, replace_str, allow_multiple)
    {
        return finalize_edit(
            &path,
            &original,
            &new_content,
            dry_run,
            path_str,
            "regex",
            count,
        );
    }

    let mut error_msg = format!(
        "Search string not found in file (tried exact, flexible, and regex match).\n\
         File: {}\n\
         Search (first 200 chars): {}",
        path_str,
        search_str
            .char_indices()
            .nth(200)
            .map(|(i, _)| &search_str[..i])
            .unwrap_or(search_str)
    );

    if search_str.contains("\\n") || replace_str.contains("\\n") {
        error_msg.push_str("\n\nPROTIP: The search or replace string contains literal '\\n' (backslash + n). \
            If you intended to represent a newline, use a real newline character in your JSON argument instead of the string \"\\n\".");
    }

    Err(anyhow::anyhow!(error_msg))
}

fn try_exact_edit(
    original: &str,
    search: &str,
    replace: &str,
    allow_multiple: bool,
) -> Option<(String, usize)> {
    let count = original.matches(search).count();
    if count == 0 {
        return None;
    }
    if !allow_multiple && count > 1 {
        // We found it but it's not unique. We return None so it can fail later or we could error here.
        // But for consistency with gemini-cli, we might want to report multiple matches if it's the only issue.
        // For now, let's just say if we can't do it uniquely, we don't do it with this strategy.
        return None;
    }

    let new_content = if allow_multiple {
        original.replace(search, replace)
    } else {
        original.replacen(search, replace, 1)
    };

    Some((new_content, count))
}

fn try_flexible_edit(
    original: &str,
    search: &str,
    replace: &str,
    allow_multiple: bool,
) -> Option<(String, usize)> {
    let source_lines: Vec<&str> = original.lines().collect();
    let search_lines: Vec<&str> = search.lines().map(|l| l.trim()).collect();
    if search_lines.is_empty() {
        return None;
    }

    let replace_lines: Vec<&str> = replace.lines().collect();
    let mut occurrences = 0;
    let mut new_lines = Vec::new();
    let mut i = 0;

    while i < source_lines.len() {
        if i + search_lines.len() <= source_lines.len() {
            let mut matched = true;
            for j in 0..search_lines.len() {
                if source_lines[i + j].trim() != search_lines[j] {
                    matched = false;
                    break;
                }
            }

            if matched {
                occurrences += 1;
                if allow_multiple || occurrences == 1 {
                    let first_line = source_lines[i];
                    let indentation: String = first_line
                        .chars()
                        .take_while(|c| c.is_whitespace())
                        .collect();
                    let indented_replace = apply_indentation(&replace_lines, &indentation);
                    for line in indented_replace {
                        new_lines.push(line);
                    }
                    i += search_lines.len();
                    continue;
                }
            }
        }
        new_lines.push(source_lines[i].to_string());
        i += 1;
    }

    if occurrences == 0 || (!allow_multiple && occurrences > 1) {
        return None;
    }

    let mut result = new_lines.join("\n");
    result = restore_trailing_newline(original, &result);
    Some((result, occurrences))
}

fn try_regex_edit(
    original: &str,
    search: &str,
    replace: &str,
    allow_multiple: bool,
) -> Option<(String, usize)> {
    let delimiters = [
        '(', ')', ':', '[', ']', '{', '}', '>', '<', '=', '.', ',', ';',
    ];
    let mut processed_search = search.to_string();
    for delim in delimiters {
        processed_search = processed_search.replace(delim, &format!(" {} ", delim));
    }

    let tokens: Vec<&str> = processed_search.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }

    let escaped_tokens: Vec<String> = tokens.iter().map(|t| regex::escape(t)).collect();
    let pattern_str = escaped_tokens.join(r"\s*");
    let final_pattern = format!(r"(?m)^([ \t]*){}", pattern_str);

    let re = Regex::new(&final_pattern).ok()?;
    let matches: Vec<_> = re.find_iter(original).collect();

    if matches.is_empty() || (!allow_multiple && matches.len() > 1) {
        return None;
    }

    let replace_lines: Vec<&str> = replace.lines().collect();
    let mut last_end = 0;
    let mut result = String::new();

    for mat in &matches {
        result.push_str(&original[last_end..mat.start()]);

        let indentation = re
            .captures(mat.as_str())
            .and_then(|caps| caps.get(1))
            .map(|m| m.as_str())
            .unwrap_or("");

        let indented_replace = apply_indentation(&replace_lines, indentation);
        result.push_str(&indented_replace.join("\n"));

        last_end = mat.end();
    }
    result.push_str(&original[last_end..]);

    let result = restore_trailing_newline(original, &result);
    Some((result, matches.len()))
}

fn apply_indentation(lines: &[&str], target_indentation: &str) -> Vec<String> {
    if lines.is_empty() {
        return Vec::new();
    }

    let reference_line = lines[0];
    let ref_indent: String = reference_line
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();

    lines
        .iter()
        .map(|line| {
            if line.trim().is_empty() {
                String::new()
            } else if line.starts_with(&ref_indent) {
                format!("{}{}", target_indentation, &line[ref_indent.len()..])
            } else {
                format!("{}{}", target_indentation, line.trim_start())
            }
        })
        .collect()
}

fn restore_trailing_newline(original: &str, modified: &str) -> String {
    let had_trailing = original.ends_with('\n');
    let has_trailing = modified.ends_with('\n');
    if had_trailing && !has_trailing {
        format!("{}\n", modified)
    } else if !had_trailing && has_trailing {
        modified.trim_end_matches('\n').to_string()
    } else {
        modified.to_string()
    }
}

fn finalize_edit(
    path: &Path,
    original: &str,
    new_content: &str,
    dry_run: bool,
    path_str: &str,
    match_type: &str,
    count: usize,
) -> anyhow::Result<Value> {
    if dry_run {
        let diff = generate_diff(original, new_content);
        let truncated_diff = crate::tools::executor_utils::truncate_output(&diff);
        return Ok(json!({
            "dry_run": true,
            "diff": truncated_diff,
            "match_type": match_type,
            "replacement_count": count,
            "message": format!("Dry run complete ({} match, {} replacements). No changes written.", match_type, count)
        }));
    }

    fs::write(path, new_content).map_err(|e| anyhow::anyhow!("Cannot write file: {}", e))?;

    let diff = generate_diff(original, new_content);
    let truncated_diff = crate::tools::executor_utils::truncate_output(&diff);

    Ok(json!({
        "success": true,
        "path": path_str,
        "diff": truncated_diff,
        "match_type": match_type,
        "replacement_count": count,
        "message": format!("File edited successfully ({} match, {} replacements).", match_type, count)
    }))
}

/// Write full content to a file. Overwrites existing files.
/// Creates parent directories if they don't exist.
pub fn create_or_overwrite_file(
    args: HashMap<String, Value>,
    config: Arc<AppConfig>,
) -> anyhow::Result<Value> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'path' is required"))?;

    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'content' is required"))?;

    let path = match crate::security::path_validator::validate_path(path_str, &config.security) {
        Ok(p) => p,
        Err(e) => return Err(anyhow::anyhow!("Security Error: {}", e)),
    };
    let existed = path.exists();

    let original = if existed {
        fs::read_to_string(&path).ok()
    } else {
        None
    };

    // Create parent directories if needed
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("Cannot create directories: {}", e))?;
    }

    fs::write(&path, content).map_err(|e| anyhow::anyhow!("Cannot write file: {}", e))?;

    let diff = if let Some(orig) = original {
        generate_diff(&orig, content)
    } else {
        // For new files, show a diff showing all lines added
        generate_diff("", content)
    };
    let truncated_diff = crate::tools::executor_utils::truncate_output(&diff);

    Ok(json!({
        "success": true,
        "path": path_str,
        "bytes_written": content.len(),
        "created": !existed,
        "diff": truncated_diff,
        "message": if existed {
            format!("File overwritten: {}", path_str)
        } else {
            format!("File created: {}", path_str)
        }
    }))
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Generate a unified diff for display purposes.
fn generate_diff(original: &str, new_content: &str) -> String {
    let orig_lines: Vec<String> = original.lines().map(|s| format!("{}\n", s)).collect();
    let new_lines: Vec<String> = new_content.lines().map(|s| format!("{}\n", s)).collect();

    let diff = difflib::unified_diff(
        &orig_lines.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        &new_lines.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        "original",
        "modified",
        "",
        "",
        3,
    );

    if diff.is_empty() {
        if original == new_content {
            return "--- original\n+++ modified\n (no changes)\n".to_string();
        } else {
            // Fallback for very small changes or whitespace differences
            return "--- original\n+++ modified\n[Content changed, but diff is empty]\n"
                .to_string();
        }
    }

    diff.join("")
}
