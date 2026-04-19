use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Edit a file by replacing a specific block of text.
/// Requires an exact match of the search string.
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
    if !original.contains(search_str) {
        return Err(anyhow::anyhow!(
            "Search string not found in file (exact match required).\n\
             File: {}\n\
             Search (first 200 chars): {}",
            path_str,
            &search_str[..search_str.len().min(200)]
        ));
    }

    let new_content = original.replacen(search_str, replace_str, 1);
    let match_type = "exact";

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
