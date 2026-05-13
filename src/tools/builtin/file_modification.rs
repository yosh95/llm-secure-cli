use crate::config::models::AppConfig;
use regex::Regex;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

/// Edit a file by replacing a specific block of text.
/// Tries multiple strategies: exact, flexible (indentation-aware), regex (whitespace-insensitive),
/// and escape-fixed (auto-corrects LLM double-escaped characters).
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
            EditMetadata {
                match_type: "exact",
                count,
                escape_fixed: false,
            },
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
            EditMetadata {
                match_type: "flexible",
                count,
                escape_fixed: false,
            },
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
            EditMetadata {
                match_type: "regex",
                count,
                escape_fixed: false,
            },
        );
    }

    // Strategy 4: Escape-fixed match
    // LLMs sometimes double-escape characters (e.g. \" instead of "), causing all
    // previous strategies to fail. We fix the escapes and retry all three strategies.
    if let (Some(fixed_search), Some(fixed_replace)) =
        (fix_llm_escapes(search_str), fix_llm_escapes(replace_str))
    {
        if let Some((new_content, count)) =
            try_exact_edit(&original, &fixed_search, &fixed_replace, allow_multiple)
                .or_else(|| {
                    try_flexible_edit(&original, &fixed_search, &fixed_replace, allow_multiple)
                })
                .or_else(|| {
                    try_regex_edit(&original, &fixed_search, &fixed_replace, allow_multiple)
                })
        {
            return finalize_edit(
                &path,
                &original,
                &new_content,
                dry_run,
                path_str,
                EditMetadata {
                    match_type: "escape-fixed",
                    count,
                    escape_fixed: true,
                },
            );
        }
    } else if let Some(fixed_search) = fix_llm_escapes(search_str) {
        // Only search had escapes, replace was fine
        if let Some((new_content, count)) =
            try_exact_edit(&original, &fixed_search, replace_str, allow_multiple)
                .or_else(|| {
                    try_flexible_edit(&original, &fixed_search, replace_str, allow_multiple)
                })
                .or_else(|| try_regex_edit(&original, &fixed_search, replace_str, allow_multiple))
        {
            return finalize_edit(
                &path,
                &original,
                &new_content,
                dry_run,
                path_str,
                EditMetadata {
                    match_type: "escape-fixed",
                    count,
                    escape_fixed: true,
                },
            );
        }
    }

    let mut error_msg = format!(
        "Search string not found in file (tried exact, flexible, regex, and escape-fixed match).\n\
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

/// Fix LLM double-escaped sequences in tool arguments.
///
/// When an LLM generates tool call arguments, it sometimes double-escapes characters
/// that should appear literally. The most common case is double-escaped quotes:
/// the LLM produces `\"` (backslash + double-quote) where it intended a literal `"`.
/// Since `serde_json` faithfully decodes the JSON string, the Rust string ends up
/// containing `\"` instead of the intended `"`, causing match failures or corrupt
/// file writes.
///
/// This function detects and fixes such patterns:
///   - `\"` → `"`  (double-escaped quote → literal quote)
///   - `\n` → newline (double-escaped newline → real newline, only when no real newlines exist)
///   - `\t` → tab (double-escaped tab → real tab, only when no real tabs exist)
///
/// Returns `Some(fixed)` if any fix was applied, `None` if the string looks correct already.
fn fix_llm_escapes(s: &str) -> Option<String> {
    // Quick check: does the string contain any potentially double-escaped sequences?
    // Note: in Rust source, `\\` in a string literal is a single backslash character.
    // So `s.contains("\\\"")` checks for the two-char sequence: backslash + double-quote.
    let has_escaped_quote = s.contains("\\\"");
    let has_escaped_newline = s.contains("\\n");
    let has_escaped_tab = s.contains("\\t");

    if !has_escaped_quote && !has_escaped_newline && !has_escaped_tab {
        return None;
    }

    // Build the fixed string by scanning byte-by-byte for backslash-prefixed sequences.
    // Using .as_bytes() handles backslash unambiguously (backslash is a single byte 0x5C).
    let bytes = s.as_bytes();
    let mut result = Vec::with_capacity(s.len());
    let mut i = 0;
    let mut changed = false;

    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            match next {
                b'"' => {
                    // `\"` → `"`
                    result.push(b'"');
                    i += 2;
                    changed = true;
                }
                b'n' => {
                    // `\n` → real newline, but only if the string has no real newlines.
                    // If real newlines already exist, the `\n` is likely intentional.
                    if !s.contains('\n') {
                        result.push(b'\n');
                        i += 2;
                        changed = true;
                    } else {
                        result.push(b'\\');
                        i += 1;
                    }
                }
                b't' => {
                    // `\t` → real tab, but only if the string has no real tabs.
                    if !s.contains('\t') {
                        result.push(b'\t');
                        i += 2;
                        changed = true;
                    } else {
                        result.push(b'\\');
                        i += 1;
                    }
                }
                b'\\' => {
                    // `\\` — an escaped backslash. Preserve both bytes as-is.
                    result.push(b'\\');
                    result.push(b'\\');
                    i += 2;
                }
                _ => {
                    // Any other `\x` — preserve as-is.
                    result.push(b'\\');
                    i += 1;
                }
            }
        } else {
            result.push(bytes[i]);
            i += 1;
        }
    }

    if changed {
        // SAFETY: we only produced valid UTF-8 sequences (single ASCII bytes or
        // passed through existing multi-byte UTF-8 unchanged)
        Some(unsafe { String::from_utf8_unchecked(result) })
    } else {
        None
    }
}

struct EditMetadata<'a> {
    match_type: &'a str,
    count: usize,
    escape_fixed: bool,
}

fn finalize_edit(
    path: &Path,
    original: &str,
    new_content: &str,
    dry_run: bool,
    path_str: &str,
    meta: EditMetadata,
) -> anyhow::Result<Value> {
    let mut message = if dry_run {
        format!(
            "Dry run complete ({} match, {} replacements). No changes written.",
            meta.match_type, meta.count
        )
    } else {
        fs::write(path, new_content).map_err(|e| anyhow::anyhow!("Cannot write file: {}", e))?;
        format!(
            "File edited successfully ({} match, {} replacements).",
            meta.match_type, meta.count
        )
    };

    if meta.escape_fixed {
        message.push_str(
            " WARNING: The search/replace strings contained double-escaped characters \
            (e.g. \\\" instead of \"). These were automatically corrected. \
            Please use raw characters in future tool calls to avoid this issue.",
        );
    }

    let diff = generate_diff(original, new_content);
    let truncated_diff = crate::tools::executor_utils::truncate_output(&diff);

    if dry_run {
        return Ok(json!({
            "dry_run": true,
            "diff": truncated_diff,
            "match_type": meta.match_type,
            "replacement_count": meta.count,
            "escape_fixed": meta.escape_fixed,
            "message": message
        }));
    }

    Ok(json!({
        "success": true,
        "path": path_str,
        "diff": truncated_diff,
        "match_type": meta.match_type,
        "replacement_count": meta.count,
        "escape_fixed": meta.escape_fixed,
        "message": message
    }))
}

/// Write full content to a file. Overwrites existing files.
/// Creates parent directories if they don't exist.
/// Automatically fixes LLM double-escaped characters in content.
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

    // Auto-fix LLM double-escaped characters
    let (effective_content, escape_fixed) = match fix_llm_escapes(content) {
        Some(fixed) => (fixed, true),
        None => (content.to_string(), false),
    };

    fs::write(&path, &effective_content)
        .map_err(|e| anyhow::anyhow!("Cannot write file: {}", e))?;

    let diff = if let Some(orig) = original {
        generate_diff(&orig, &effective_content)
    } else {
        // For new files, show a diff showing all lines added
        generate_diff("", &effective_content)
    };
    let truncated_diff = crate::tools::executor_utils::truncate_output(&diff);

    let mut message = if existed {
        format!("File overwritten: {}", path_str)
    } else {
        format!("File created: {}", path_str)
    };

    if escape_fixed {
        message.push_str(
            " WARNING: The content contained double-escaped characters \
            (e.g. \\\" instead of \"). These were automatically corrected. \
            Please use raw characters in future tool calls to avoid this issue.",
        );
    }

    Ok(json!({
        "success": true,
        "path": path_str,
        "bytes_written": effective_content.len(),
        "created": !existed,
        "escape_fixed": escape_fixed,
        "diff": truncated_diff,
        "message": message
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fix_llm_escapes_double_quotes() {
        // Input Rust string: \"hello\" (backslash-quote-hello-backslash-quote)
        // Should become: "hello"
        let input = "\\\"hello\\\"";
        let fixed = fix_llm_escapes(input);
        assert!(fixed.is_some());
        assert_eq!(fixed.unwrap(), "\"hello\"");
    }

    #[test]
    fn test_fix_llm_escapes_mixed() {
        // Mix of escaped quotes and normal text
        let input = "fn main() { println!(\\\"test\\\"); }";
        let fixed = fix_llm_escapes(input);
        assert!(fixed.is_some());
        assert_eq!(fixed.unwrap(), "fn main() { println!(\"test\"); }");
    }

    #[test]
    fn test_fix_llm_escapes_newline_without_real_newlines() {
        // \n in a string with no real newlines → convert to real newline
        let input = "line1\\nline2";
        let fixed = fix_llm_escapes(input);
        assert!(fixed.is_some());
        assert_eq!(fixed.unwrap(), "line1\nline2");
    }

    #[test]
    fn test_fix_llm_escapes_newline_with_real_newlines() {
        // \n in a string that already has real newlines → leave \n as-is
        let input = "line1\nline2\\nline3";
        let fixed = fix_llm_escapes(input);
        // The \n should NOT be converted because real newlines exist
        assert!(fixed.is_none());
    }

    #[test]
    fn test_fix_llm_escapes_no_changes_needed() {
        let input = "normal text without escapes";
        assert!(fix_llm_escapes(input).is_none());
    }

    #[test]
    fn test_fix_llm_escapes_intentional_backslash() {
        // Intentional backslash followed by non-special char (not n, t, ", \) → preserve
        // Note: \t and \n ARE recognized as double-escaped by fix_llm_escapes,
        // which is the intended behavior — those ARE the patterns LLMs double-escape.
        let input = "path\\xfile";
        assert!(fix_llm_escapes(input).is_none());
    }

    #[test]
    fn test_fix_llm_escapes_double_backslash() {
        // Double backslash \\ → preserved as \\
        let input = "escaped\\\\backslash";
        // Contains \\ but that's the double-backslash case, not a recognized
        // double-escaped pattern, so no change
        assert!(fix_llm_escapes(input).is_none());
    }

    #[test]
    fn test_fix_llm_escapes_tab_without_real_tabs() {
        let input = "col1\\tcol2";
        let fixed = fix_llm_escapes(input);
        assert!(fixed.is_some());
        assert_eq!(fixed.unwrap(), "col1\tcol2");
    }

    #[test]
    fn test_fix_llm_escapes_complex_json_like() {
        // Simulates what LLM might generate for a search string containing JSON
        let input = "{\\\"key\\\": \\\"value\\\"}";
        let fixed = fix_llm_escapes(input);
        assert!(fixed.is_some());
        assert_eq!(fixed.unwrap(), "{\"key\": \"value\"}");
    }
}
