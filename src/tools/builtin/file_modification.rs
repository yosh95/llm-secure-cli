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

    let old_str = args
        .get("old")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'old' is required"))?;

    let new_str = args
        .get("new")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'new' is required"))?;

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

    if old_str.is_empty() {
        return Err(anyhow::anyhow!(
            "Search string ('old') cannot be empty. To create or overwrite a file, use create_or_overwrite_file tool or provide a non-empty search string."
        ));
    }

    // Strategy 1: Exact match
    if let Some((new_content, count)) = try_exact_edit(&original, old_str, new_str) {
        return finalize_edit(
            &path,
            &original,
            &new_content,
            EditMetadata {
                match_type: "exact",
                count,
                escape_fixed: false,
                dry_run,
                max_output_lines: config.general.max_output_lines,
                max_output_chars: config.general.max_output_chars,
            },
        );
    }

    // Strategy 2: Flexible match (indentation-aware, line-by-line trim match)
    if let Some((new_content, count)) = try_flexible_edit(&original, old_str, new_str) {
        return finalize_edit(
            &path,
            &original,
            &new_content,
            EditMetadata {
                match_type: "flexible",
                count,
                escape_fixed: false,
                dry_run,
                max_output_lines: config.general.max_output_lines,
                max_output_chars: config.general.max_output_chars,
            },
        );
    }

    // Strategy 2.5: Partial-line match (line-by-line substring match).
    // LLMs often provide a fragment of a line (e.g. "let y = 2;" when the
    // real line is "    let y = 2; // comment").  The flexible strategy
    // requires whole-line trim equality, which fails here.  This strategy
    // checks whether each (trimmed) old line is a *substring* of the
    // corresponding (trimmed) source line.
    if let Some((new_content, count)) = try_partial_line_edit(&original, old_str, new_str) {
        return finalize_edit(
            &path,
            &original,
            &new_content,
            EditMetadata {
                match_type: "partial",
                count,
                escape_fixed: false,
                dry_run,
                max_output_lines: config.general.max_output_lines,
                max_output_chars: config.general.max_output_chars,
            },
        );
    }

    // Strategy 3: Regex match (whitespace-insensitive)
    if let Some((new_content, count)) = try_regex_edit(&original, old_str, new_str) {
        return finalize_edit(
            &path,
            &original,
            &new_content,
            EditMetadata {
                match_type: "regex",
                count,
                escape_fixed: false,
                dry_run,
                max_output_lines: config.general.max_output_lines,
                max_output_chars: config.general.max_output_chars,
            },
        );
    }

    // Strategy 4: Escape-fixed match
    // LLMs sometimes double-escape characters (e.g. \" instead of "), causing all
    // previous strategies to fail. We fix the escapes and retry all three strategies.
    if let (Some(fixed_old), Some(fixed_new)) = (fix_llm_escapes(old_str), fix_llm_escapes(new_str))
    {
        if let Some((new_content, count)) = try_exact_edit(&original, &fixed_old, &fixed_new)
            .or_else(|| try_flexible_edit(&original, &fixed_old, &fixed_new))
            .or_else(|| try_partial_line_edit(&original, &fixed_old, &fixed_new))
            .or_else(|| try_regex_edit(&original, &fixed_old, &fixed_new))
        {
            return finalize_edit(
                &path,
                &original,
                &new_content,
                EditMetadata {
                    match_type: "escape-fixed",
                    count,
                    escape_fixed: true,
                    dry_run,
                    max_output_lines: config.general.max_output_lines,
                    max_output_chars: config.general.max_output_chars,
                },
            );
        }
    } else if let Some(fixed_old) = fix_llm_escapes(old_str) {
        // Only search had escapes, replace was fine
        if let Some((new_content, count)) = try_exact_edit(&original, &fixed_old, new_str)
            .or_else(|| try_flexible_edit(&original, &fixed_old, new_str))
            .or_else(|| try_partial_line_edit(&original, &fixed_old, new_str))
            .or_else(|| try_regex_edit(&original, &fixed_old, new_str))
        {
            return finalize_edit(
                &path,
                &original,
                &new_content,
                EditMetadata {
                    match_type: "escape-fixed",
                    count,
                    escape_fixed: true,
                    dry_run,
                    max_output_lines: config.general.max_output_lines,
                    max_output_chars: config.general.max_output_chars,
                },
            );
        }
    }

    let mut error_msg = format!(
        "Search string ('old') not found in file (tried exact, flexible, partial, regex, and escape-fixed match).\n\
         File: {}\n\
         Old (first 200 chars): {}",
        path_str,
        old_str
            .char_indices()
            .nth(200)
            .map(|(i, _)| &old_str[..i])
            .unwrap_or(old_str)
    );

    if old_str.contains("\\n") || new_str.contains("\\n") {
        error_msg.push_str("\n\nPROTIP: The 'old' or 'new' string contains literal '\\n' (backslash + n). \
            If you intended to represent a newline, use a real newline character in your JSON argument instead of the string \"\\n\".");
    }

    Err(anyhow::anyhow!(error_msg))
}

fn try_exact_edit(original: &str, old: &str, new: &str) -> Option<(String, usize)> {
    let count = original.matches(old).count();
    if count != 1 {
        return None;
    }

    let new_content = original.replacen(old, new, 1);

    Some((new_content, count))
}

fn try_flexible_edit(original: &str, old: &str, new: &str) -> Option<(String, usize)> {
    let source_lines: Vec<&str> = original.lines().collect();
    let old_lines: Vec<&str> = old.lines().map(|l| l.trim()).collect();
    if old_lines.is_empty() {
        return None;
    }

    let new_lines: Vec<&str> = new.lines().collect();
    let mut occurrences = 0;
    let mut new_lines_result = Vec::new();
    let mut i = 0;

    while i < source_lines.len() {
        if i + old_lines.len() <= source_lines.len() {
            let mut matched = true;
            for j in 0..old_lines.len() {
                if source_lines[i + j].trim() != old_lines[j] {
                    matched = false;
                    break;
                }
            }

            if matched {
                occurrences += 1;
                if occurrences == 1 {
                    let first_line = source_lines[i];
                    let indentation: String = first_line
                        .chars()
                        .take_while(|c| c.is_whitespace())
                        .collect();
                    let indented_new = apply_indentation(&new_lines, &indentation);
                    for line in indented_new {
                        new_lines_result.push(line);
                    }
                    i += old_lines.len();
                    continue;
                }
            }
        }
        new_lines_result.push(source_lines[i].to_string());
        i += 1;
    }

    if occurrences != 1 {
        return None;
    }

    let mut result = new_lines_result.join("\n");
    result = restore_trailing_newline(original, &result);
    Some((result, occurrences))
}

/// Like `try_flexible_edit`, but matches when each (trimmed) old line is a
/// *substring* of the corresponding (trimmed) source line rather than an
/// exact equality.  This handles cases where the LLM picks a fragment of a
/// long line (e.g. missing a trailing comment, or only the code portion).
fn try_partial_line_edit(original: &str, old: &str, new: &str) -> Option<(String, usize)> {
    let source_lines: Vec<&str> = original.lines().collect();
    let old_lines: Vec<&str> = old.lines().map(|l| l.trim()).collect();
    if old_lines.is_empty() {
        return None;
    }
    // Avoid ambiguity: if old equals the whole trimmed line everywhere,
    // the flexible strategy already handles it; this strategy is for
    // *strict substrings* only.
    let is_proper_substring = old_lines.iter().enumerate().any(|(j, ol)| {
        if j < source_lines.len() {
            let st = source_lines[j].trim();
            st != *ol && st.contains(*ol)
        } else {
            false
        }
    });
    if !is_proper_substring {
        return None;
    }

    let new_lines: Vec<&str> = new.lines().collect();
    let mut occurrences = 0;
    let mut new_lines_result = Vec::new();
    let mut i = 0;

    while i < source_lines.len() {
        if i + old_lines.len() <= source_lines.len() {
            let mut matched = true;
            for j in 0..old_lines.len() {
                if !source_lines[i + j].trim().contains(old_lines[j]) {
                    matched = false;
                    break;
                }
            }

            if matched {
                occurrences += 1;
                if occurrences == 1 {
                    // For partial-line matches we replace the *entire* source
                    // line with the corresponding new line, preserving the
                    // original indentation.
                    let first_line = source_lines[i];
                    let indentation: String = first_line
                        .chars()
                        .take_while(|c| c.is_whitespace())
                        .collect();
                    let indented_new = apply_indentation(&new_lines, &indentation);
                    for line in indented_new {
                        new_lines_result.push(line);
                    }
                    i += old_lines.len();
                    continue;
                }
            }
        }
        new_lines_result.push(source_lines[i].to_string());
        i += 1;
    }

    if occurrences != 1 {
        return None;
    }

    let mut result = new_lines_result.join("\n");
    result = restore_trailing_newline(original, &result);
    Some((result, occurrences))
}

fn try_regex_edit(original: &str, old: &str, new: &str) -> Option<(String, usize)> {
    let delimiters = [
        '(', ')', ':', '[', ']', '{', '}', '>', '<', '=', '.', ',', ';',
    ];
    let mut processed_old = old.to_string();
    for delim in delimiters {
        processed_old = processed_old.replace(delim, &format!(" {} ", delim));
    }

    let tokens: Vec<&str> = processed_old.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }

    let escaped_tokens: Vec<String> = tokens.iter().map(|t| regex::escape(t)).collect();
    let pattern_str = escaped_tokens.join(r"\s*");
    let final_pattern = format!(r"(?m)^([ \t]*){}", pattern_str);

    let re = Regex::new(&final_pattern).ok()?;
    let matches: Vec<_> = re.find_iter(original).collect();

    if matches.len() != 1 {
        return None;
    }

    let new_lines: Vec<&str> = new.lines().collect();
    let mut last_end = 0;
    let mut result = String::new();

    for mat in &matches {
        result.push_str(&original[last_end..mat.start()]);

        let indentation = re
            .captures(mat.as_str())
            .and_then(|caps| caps.get(1))
            .map(|m| m.as_str())
            .unwrap_or_default();

        let indented_new = apply_indentation(&new_lines, indentation);
        result.push_str(&indented_new.join("\n"));

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
///   - `\n` → newline (double-escaped newline → real newline)
///   - `\t` → tab (double-escaped tab → real tab)
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
                    // `\n` → real newline. LLMs almost always intend a newline
                    // character when they write \n, but JSON serialisation
                    // double-escapes it.  If the caller truly wants a literal
                    // backslash+n they should write \\n.
                    result.push(b'\n');
                    i += 2;
                    changed = true;
                }
                b't' => {
                    // `\t` → real tab (same reasoning as \n above).
                    result.push(b'\t');
                    i += 2;
                    changed = true;
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
        String::from_utf8(result).ok()
    } else {
        None
    }
}

struct EditMetadata<'a> {
    match_type: &'a str,
    count: usize,
    escape_fixed: bool,
    dry_run: bool,
    max_output_lines: usize,
    max_output_chars: usize,
}

fn finalize_edit(
    path: &Path,
    original: &str,
    new_content: &str,
    meta: EditMetadata,
) -> anyhow::Result<Value> {
    let path_str = path.display().to_string();

    // Guard: detect when the edit would produce no actual change.
    // This catches cases like old == new, or whitespace-only diffs
    // that collapse to identical content.
    if original == new_content {
        return Err(anyhow::anyhow!(
            "Edit produced no changes: the 'old' string was found, but the 'new' replacement \
            is identical to the matched text. The file was not modified.\n\
            File: {}",
            path_str,
        ));
    }

    let mut message = if meta.dry_run {
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
            " WARNING: The 'old'/'new' strings contained double-escaped characters \
            (e.g. \\\" instead of \"). These were automatically corrected. \
            Please use raw characters in future tool calls to avoid this issue.",
        );
    }

    let diff = generate_diff(original, new_content);
    let truncated_diff = crate::tools::executor_utils::truncate_output(
        &diff,
        meta.max_output_lines,
        meta.max_output_chars,
    );

    if meta.dry_run {
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
    let truncated_diff = crate::tools::executor_utils::truncate_output(
        &diff,
        config.general.max_output_lines,
        config.general.max_output_chars,
    );

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
///
/// This is a native Rust implementation that replaces the `difflib` crate,
/// removing a transitive Python-heritage dependency.  Uses a classic
/// patience/LCS diff algorithm and outputs standard unified-diff format
/// (compatible with `patch(1)`).
pub fn generate_diff(original: &str, new_content: &str) -> String {
    let orig_lines: Vec<&str> = original.lines().collect();
    let new_lines: Vec<&str> = new_content.lines().collect();

    // Compute the edit script using an LCS-based diff.
    let hunks = compute_unified_hunks(&orig_lines, &new_lines, 3);

    if hunks.is_empty() {
        if original == new_content {
            return "--- original\n+++ modified\n (no changes)\n".to_string();
        } else {
            return "--- original\n+++ modified\n[Content changed, but diff is empty]\n"
                .to_string();
        }
    }

    let mut result = String::from("--- original\n+++ modified\n");
    for hunk in &hunks {
        result.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            hunk.old_start, hunk.old_count, hunk.new_start, hunk.new_count
        ));
        for line in &hunk.lines {
            result.push_str(line);
            result.push('\n');
        }
    }
    result
}

/// A single hunk in a unified diff.
struct Hunk {
    old_start: usize,
    old_count: usize,
    new_start: usize,
    new_count: usize,
    lines: Vec<String>,
}

/// Compute unified-diff hunks between two line slices with `context` lines
/// of surrounding context.
fn compute_unified_hunks(old: &[&str], new: &[&str], context: usize) -> Vec<Hunk> {
    let edits = lcs_diff(old, new);
    if edits.is_empty() {
        return Vec::new();
    }

    // Group edits into hunks, merging hunks that overlap within `context` lines.
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut i = 0;
    while i < edits.len() {
        let start_old = edits[i].old_pos.saturating_sub(context);
        let start_new = edits[i].new_pos.saturating_sub(context);
        let mut end_idx = i;

        // Expand the hunk to merge nearby changes
        loop {
            let next_old_end = if end_idx + 1 < edits.len() {
                edits[end_idx + 1].old_pos + context
            } else {
                old.len()
            };
            let cur_old_end = edits[end_idx].old_pos;
            if end_idx + 1 < edits.len() && next_old_end <= cur_old_end + context + 1 {
                end_idx += 1;
            } else {
                break;
            }
        }

        let end_old = std::cmp::min(edits[end_idx].old_pos + context + 1, old.len());
        let end_new = std::cmp::min(edits[end_idx].new_pos + context + 1, new.len());

        let mut lines = Vec::new();
        let mut oi = start_old;
        let mut ni = start_new;

        while oi < end_old || ni < end_new {
            // Find next edit at or before current position
            let next_edit_old = edits
                .iter()
                .find(|e| e.old_pos == oi && e.kind != EditKind::Insert);
            let next_edit_new = edits
                .iter()
                .find(|e| e.new_pos == ni && e.kind == EditKind::Insert);

            if let Some(e) = next_edit_old {
                match e.kind {
                    EditKind::Delete => {
                        lines.push(format!("-{}", old[oi]));
                        oi += 1;
                        continue;
                    }
                    EditKind::Replace => {
                        lines.push(format!("-{}", old[oi]));
                        lines.push(format!("+{}", new[ni]));
                        oi += 1;
                        ni += 1;
                        continue;
                    }
                    EditKind::Insert => unreachable!(),
                    EditKind::Keep => {}
                }
            } else if let Some(_e) = next_edit_new {
                lines.push(format!("+{}", new[ni]));
                ni += 1;
                continue;
            }

            // Context line
            if oi < old.len() && ni < new.len() && old[oi] == new[ni] {
                lines.push(format!(" {}", old[oi]));
                oi += 1;
                ni += 1;
            } else if oi < end_old && (ni >= end_new || oi < old.len()) {
                lines.push(format!("-{}", old[oi]));
                oi += 1;
            } else if ni < end_new {
                lines.push(format!("+{}", new[ni]));
                ni += 1;
            } else {
                break;
            }
        }

        hunks.push(Hunk {
            old_start: start_old + 1, // 1-indexed
            old_count: oi.saturating_sub(start_old),
            new_start: start_new + 1,
            new_count: ni.saturating_sub(start_new),
            lines,
        });

        i = end_idx + 1;
    }

    hunks
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditKind {
    #[allow(dead_code)]
    Keep,
    Delete,
    Insert,
    #[allow(dead_code)]
    Replace,
}

struct Edit {
    old_pos: usize,
    new_pos: usize,
    kind: EditKind,
}

/// Compute the diff between two line slices using LCS.
/// Returns only the non-keep edit operations.
fn lcs_diff(old: &[&str], new: &[&str]) -> Vec<Edit> {
    if old.is_empty() && new.is_empty() {
        return Vec::new();
    }
    if old.is_empty() {
        return new
            .iter()
            .enumerate()
            .map(|(i, _)| Edit {
                old_pos: 0,
                new_pos: i,
                kind: EditKind::Insert,
            })
            .collect();
    }
    if new.is_empty() {
        return old
            .iter()
            .enumerate()
            .map(|(i, _)| Edit {
                old_pos: i,
                new_pos: 0,
                kind: EditKind::Delete,
            })
            .collect();
    }

    // Build LCS table
    let m = old.len();
    let n = new.len();
    let mut table = vec![vec![0usize; n + 1]; m + 1];

    for i in 1..=m {
        for j in 1..=n {
            if old[i - 1] == new[j - 1] {
                table[i][j] = table[i - 1][j - 1] + 1;
            } else {
                table[i][j] = std::cmp::max(table[i - 1][j], table[i][j - 1]);
            }
        }
    }

    // Backtrack to produce the edit script
    let mut edits = Vec::new();
    let mut i = m;
    let mut j = n;

    while i > 0 || j > 0 {
        if i > 0 && j > 0 && old[i - 1] == new[j - 1] {
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || table[i][j - 1] >= table[i - 1][j]) {
            j -= 1;
            edits.push(Edit {
                old_pos: i,
                new_pos: j,
                kind: EditKind::Insert,
            });
        } else if i > 0 {
            i -= 1;
            edits.push(Edit {
                old_pos: i,
                new_pos: j,
                kind: EditKind::Delete,
            });
        }
    }

    edits.reverse();
    edits
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_fix_llm_escapes_double_quotes() {
        // Input Rust string: \"hello\" (backslash-quote-hello-backslash-quote)
        // Should become: "hello"
        let input = "\\\"hello\\\"";
        let fixed = fix_llm_escapes(input);
        assert!(fixed.is_some());
        assert_eq!(fixed.expect("Expected Some"), "\"hello\"");
    }

    #[test]
    fn test_fix_llm_escapes_mixed() {
        // Mix of escaped quotes and normal text
        let input = "fn main() { println!(\\\"test\\\"); }";
        let fixed = fix_llm_escapes(input);
        assert!(fixed.is_some());
        assert_eq!(
            fixed.expect("Expected Some"),
            "fn main() { println!(\"test\"); }"
        );
    }

    #[test]
    fn test_fix_llm_escapes_newline_without_real_newlines() {
        // \n in a string with no real newlines → convert to real newline
        let input = "line1\\nline2";
        let fixed = fix_llm_escapes(input);
        assert!(fixed.is_some());
        assert_eq!(fixed.expect("Expected Some"), "line1\nline2");
    }

    #[test]
    fn test_fix_llm_escapes_newline_with_real_newlines() {
        // \n in a string that already has real newlines → still convert
        // because LLMs routinely mix both forms and we now always unescape.
        let input = "line1\nline2\\nline3";
        let fixed = fix_llm_escapes(input);
        assert!(fixed.is_some());
        // The \\n was converted to a real newline
        assert_eq!(fixed.expect("Expected Some"), "line1\nline2\nline3");
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
        assert_eq!(fixed.expect("Expected Some"), "col1\tcol2");
    }

    #[test]
    fn test_fix_llm_escapes_complex_json_like() {
        // Simulates what LLM might generate for a search string containing JSON
        let input = "{\\\"key\\\": \\\"value\\\"}";
        let fixed = fix_llm_escapes(input);
        assert!(fixed.is_some());
        assert_eq!(fixed.expect("Expected Some"), "{\"key\": \"value\"}");
    }
}
