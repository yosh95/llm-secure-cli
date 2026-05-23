pub fn truncate_output(res_str: &str, max_output_lines: usize, max_output_chars: usize) -> String {
    let original_len = res_str.len();
    let original_lines: Vec<&str> = res_str.lines().collect();
    let original_lines_count = original_lines.len();

    if original_lines_count > max_output_lines || original_len > max_output_chars {
        let mut truncated_lines = original_lines;
        truncated_lines.truncate(max_output_lines);
        let mut joined = truncated_lines.join("\n");
        if joined.len() > max_output_chars {
            let mut new_len = max_output_chars;
            while new_len > 0 && !joined.is_char_boundary(new_len) {
                new_len -= 1;
            }
            joined.truncate(new_len);
        }

        let shown_lines_count = joined.lines().count();
        let shown_chars = joined.len();

        joined.push_str(&format!(
            "\n\n... (Output truncated. Shown {} of {} lines, {} of {} chars.)",
            crate::utils::format_number(shown_lines_count),
            crate::utils::format_number(original_lines_count),
            crate::utils::format_number(shown_chars),
            crate::utils::format_number(original_len)
        ));
        joined
    } else {
        res_str.to_string()
    }
}

pub fn truncate_json_strings(
    v: &mut serde_json::Value,
    max_output_lines: usize,
    max_output_chars: usize,
) {
    match v {
        serde_json::Value::String(s) if s.len() > max_output_chars => {
            *s = truncate_output(s, max_output_lines, max_output_chars);
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                truncate_json_strings(item, max_output_lines, max_output_chars);
            }
        }
        serde_json::Value::Object(obj) => {
            for (_, value) in obj {
                truncate_json_strings(value, max_output_lines, max_output_chars);
            }
        }
        _ => {}
    }
}

/// Converts a tool result (JSON) into a human-readable string.
/// This is used to provide better output for both humans (CLI) and LLMs.
pub fn humanize_tool_result(_name: &str, v: &serde_json::Value) -> String {
    if let Some(obj) = v.as_object() {
        // Special handling for brave_search results
        if let Some(results) = obj.get("results").and_then(|v| v.as_array())
            && obj.get("query").is_some()
        {
            if results.is_empty() {
                return "No search results found.".to_string();
            }
            let query = obj
                .get("query")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let mut output = format!(
                "Search results for \"{}\" ({} items):\n\n",
                query,
                results.len()
            );
            for (i, item) in results.iter().enumerate() {
                let title = item
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let url = item.get("url").and_then(|v| v.as_str()).unwrap_or_default();
                let snippets = item.get("snippets").and_then(|v| v.as_array());

                output.push_str(&format!("{}. {}\n", i + 1, title));
                output.push_str(&format!("   URL: {}\n", url));
                if let Some(snip_arr) = snippets {
                    for snippet in snip_arr {
                        if let Some(s) = snippet.as_str() {
                            output.push_str(&format!("   {}\n", s));
                        }
                    }
                }
                output.push('\n');
            }
            return output;
        }

        // Special handling for command execution
        if obj.contains_key("stdout") && obj.contains_key("stderr") && obj.contains_key("exit_code")
        {
            let stdout = obj
                .get("stdout")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let stderr = obj
                .get("stderr")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let exit_code = obj.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(-1);

            let mut output = format!("Exit Code: {}\n", exit_code);
            if !stdout.is_empty() {
                output.push_str("STDOUT:\n");
                output.push_str(stdout);
                if !stdout.ends_with('\n') {
                    output.push('\n');
                }
            }
            if !stderr.is_empty() {
                output.push_str("STDERR:\n");
                output.push_str(stderr);
                if !stderr.ends_with('\n') {
                    output.push('\n');
                }
            }
            return output;
        }
    }

    // Fallback: use pretty-printed JSON if it's a complex object, otherwise as_str or to_string
    if v.is_object() || v.is_array() {
        serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
    } else if let Some(s) = v.as_str() {
        s.to_string()
    } else {
        v.to_string()
    }
}
