use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Edit a file by replacing a specific block of text.
/// First tries exact match, then falls back to fuzzy match
/// (ignoring minor whitespace and indentation differences).
pub fn edit_file(args: HashMap<String, Value>) -> anyhow::Result<Value> {
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

    let dry_run = args
        .get("dry_run")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let path = Path::new(path_str);
    if !path.exists() {
        return Err(anyhow::anyhow!("File not found: {}", path_str));
    }

    let original =
        fs::read_to_string(path).map_err(|e| anyhow::anyhow!("Cannot read file: {}", e))?;

    // 1. Try exact match
    let (new_content, match_type) = if original.contains(search_str) {
        let replaced = original.replacen(search_str, replace_str, 1);
        (replaced, "exact")
    } else {
        // 2. Fuzzy match: normalize whitespace/indentation
        let normalized_original = normalize_whitespace(&original);
        let normalized_search = normalize_whitespace(search_str);

        if normalized_original.contains(&normalized_search) {
            let restored = replace_with_indentation(
                &original,
                &normalized_original,
                search_str,
                replace_str,
                &normalized_search,
            );
            (restored, "fuzzy")
        } else {
            return Err(anyhow::anyhow!(
                "Search string not found in file (neither exact nor fuzzy match).\n\
                 File: {}\n\
                 Search (first 200 chars): {}",
                path_str,
                &search_str[..search_str.len().min(200)]
            ));
        }
    };

    if dry_run {
        let diff = generate_diff(&original, &new_content);
        return Ok(json!({
            "dry_run": true,
            "match_type": match_type,
            "diff": diff,
            "message": format!("Dry run complete ({} match). No changes written.", match_type)
        }));
    }

    fs::write(path, &new_content).map_err(|e| anyhow::anyhow!("Cannot write file: {}", e))?;

    Ok(json!({
        "success": true,
        "match_type": match_type,
        "path": path_str,
        "message": format!("File edited successfully ({} match).", match_type)
    }))
}

/// Write full content to a file. Overwrites existing files.
/// Creates parent directories if they don't exist.
pub fn create_or_overwrite_file(args: HashMap<String, Value>) -> anyhow::Result<Value> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'path' is required"))?;

    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'content' is required"))?;

    let path = Path::new(path_str);

    // Create parent directories if needed
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| anyhow::anyhow!("Cannot create directories: {}", e))?;
        }
    }

    let existed = path.exists();
    fs::write(path, content).map_err(|e| anyhow::anyhow!("Cannot write file: {}", e))?;

    Ok(json!({
        "success": true,
        "path": path_str,
        "bytes_written": content.len(),
        "created": !existed,
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

/// Normalize whitespace: trim leading/trailing space on each line.
fn normalize_whitespace(s: &str) -> String {
    s.lines()
        .map(|line| line.trim())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Replace the normalized search block in the original file content,
/// attempting to preserve the original indentation of the replace block.
fn replace_with_indentation(
    original: &str,
    normalized_original: &str,
    search: &str,
    replace: &str,
    normalized_search: &str,
) -> String {
    // Detect the indentation of the search block's first non-empty line
    let base_indent: String = search
        .lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| {
            let trimmed = l.trim_start();
            l[..l.len() - trimmed.len()].to_string()
        })
        .unwrap_or_default();

    // Re-indent the replace block using the detected base indent
    let indented_replace: String = replace
        .lines()
        .enumerate()
        .map(|(i, line)| {
            if line.trim().is_empty() {
                line.to_string()
            } else if i == 0 {
                // First line inherits base indent
                format!("{}{}", base_indent, line.trim_start())
            } else {
                // Subsequent lines: preserve relative indentation
                let extra_indent = line.chars().take_while(|c| c.is_whitespace()).count();
                let base_extra = replace
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .map(|l| l.chars().take_while(|c| c.is_whitespace()).count())
                    .unwrap_or(0);
                let relative = extra_indent.saturating_sub(base_extra);
                format!(
                    "{}{}{}",
                    base_indent,
                    " ".repeat(relative),
                    line.trim_start()
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Find the position in the normalized original and map back to line ranges
    if let Some(norm_pos) = normalized_original.find(normalized_search) {
        let before_norm = &normalized_original[..norm_pos];
        let norm_search_line_count = normalized_search.lines().count();
        let before_line_count = before_norm.lines().count();

        // Account for the edge case where before_norm is empty
        let before_line_count = if norm_pos == 0 { 0 } else { before_line_count };

        let orig_lines: Vec<&str> = original.lines().collect();
        let search_line_count = search.lines().count();

        let before_part = orig_lines[..before_line_count.min(orig_lines.len())].join("\n");
        let after_start = (before_line_count + search_line_count.max(norm_search_line_count))
            .min(orig_lines.len());
        let after_part = orig_lines[after_start..].join("\n");

        let mut result = String::new();
        if !before_part.is_empty() {
            result.push_str(&before_part);
            result.push('\n');
        }
        result.push_str(&indented_replace);
        if !after_part.is_empty() {
            result.push('\n');
            result.push_str(&after_part);
        }

        // Preserve trailing newline if original had one
        if original.ends_with('\n') && !result.ends_with('\n') {
            result.push('\n');
        }

        result
    } else {
        // Fallback: just do a normalized-level replacement
        normalized_original.replacen(normalized_search, &normalize_whitespace(replace), 1)
    }
}

/// Generate a simple line-based diff for display purposes.
fn generate_diff(original: &str, new_content: &str) -> String {
    let orig_lines: Vec<&str> = original.lines().collect();
    let new_lines: Vec<&str> = new_content.lines().collect();

    let mut diff = String::new();
    diff.push_str("--- original\n");
    diff.push_str("+++ modified\n");

    let max_len = orig_lines.len().max(new_lines.len());
    for i in 0..max_len {
        match (orig_lines.get(i), new_lines.get(i)) {
            (Some(o), Some(n)) if o == n => {
                diff.push_str(&format!(" {}\n", o));
            }
            (Some(o), Some(n)) => {
                diff.push_str(&format!("-{}\n", o));
                diff.push_str(&format!("+{}\n", n));
            }
            (Some(o), None) => {
                diff.push_str(&format!("-{}\n", o));
            }
            (None, Some(n)) => {
                diff.push_str(&format!("+{}\n", n));
            }
            (None, None) => {}
        }
    }

    diff
}
