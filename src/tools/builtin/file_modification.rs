use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Edit a file by replacing a specific block of text.
/// First tries an exact match. If that fails, tries a fuzzy match (ignoring leading/trailing whitespace on each line).
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
    if original.contains(search_str) {
        let new_content = original.replacen(search_str, replace_str, 1);
        return finalize_edit(path, &original, &new_content, dry_run, path_str, "exact");
    }

    // 2. Try fuzzy match (line-by-line, ignoring leading/trailing whitespace)
    let search_lines: Vec<&str> = search_str.lines().map(|l| l.trim()).collect();
    if search_lines.is_empty() {
        // If search_str is just whitespace/newlines, we don't want to fuzzy match it
        return Err(anyhow::anyhow!(
            "Search string not found (exact). Fuzzy match skipped because search string is empty or only whitespace."
        ));
    }

    let file_lines_raw: Vec<&str> = original.lines().collect();
    let mut matches = Vec::new();

    if file_lines_raw.len() >= search_lines.len() {
        for i in 0..=(file_lines_raw.len() - search_lines.len()) {
            let mut matched = true;
            for j in 0..search_lines.len() {
                if file_lines_raw[i + j].trim() != search_lines[j] {
                    matched = false;
                    break;
                }
            }
            if matched {
                matches.push(i);
            }
        }
    }

    if matches.len() == 1 {
        let start_line_idx = matches[0];

        // Find byte offsets for the matched lines to preserve everything else
        let mut start_byte = 0;
        let mut current_line = 0;
        let mut it = original.char_indices();

        if start_line_idx > 0 {
            for (idx, c) in it.by_ref() {
                if c == '\n' {
                    current_line += 1;
                    if current_line == start_line_idx {
                        start_byte = idx + 1;
                        break;
                    }
                }
            }
        }

        let mut end_byte = original.len();
        let mut lines_in_match = 0;
        // it continues from where it left off
        for (idx, c) in it {
            if c == '\n' {
                lines_in_match += 1;
                if lines_in_match == search_lines.len() {
                    end_byte = idx + 1;
                    break;
                }
            }
        }
        // If we reached the end of search lines but not the end of file and no trailing \n in matched block
        if lines_in_match < search_lines.len()
            && start_line_idx + search_lines.len() == file_lines_raw.len()
        {
            end_byte = original.len();
        }

        let mut new_content = String::with_capacity(original.len() + replace_str.len());
        new_content.push_str(&original[..start_byte]);
        new_content.push_str(replace_str);

        // If we replaced a block that ended with a newline but replace_str doesn't,
        // and it wasn't the very end of the file, we might want to keep the newline.
        // However, usually the agent provides the intended structure.
        // To be safe and minimize "indentation collapse" or "formatting loss",
        // we'll just stick to replacing the exact line range found.

        new_content.push_str(&original[end_byte..]);

        return finalize_edit(path, &original, &new_content, dry_run, path_str, "fuzzy");
    }

    if matches.len() > 1 {
        return Err(anyhow::anyhow!(
            "Multiple fuzzy matches found ({}) for the search string. Please provide more context to make it unique.",
            matches.len()
        ));
    }

    Err(anyhow::anyhow!(
        "Search string not found in file (tried exact and fuzzy match).\n\
         File: {}\n\
         Search (first 200 chars): {}",
        path_str,
        &search_str[..search_str.len().min(200)]
    ))
}

fn finalize_edit(
    path: &Path,
    original: &str,
    new_content: &str,
    dry_run: bool,
    path_str: &str,
    match_type: &str,
) -> anyhow::Result<Value> {
    if dry_run {
        let diff = generate_diff(original, new_content);
        return Ok(json!({
            "dry_run": true,
            "diff": diff,
            "match_type": match_type,
            "message": format!("Dry run complete ({} match). No changes written.", match_type)
        }));
    }

    fs::write(path, new_content).map_err(|e| anyhow::anyhow!("Cannot write file: {}", e))?;

    Ok(json!({
        "success": true,
        "path": path_str,
        "match_type": match_type,
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
