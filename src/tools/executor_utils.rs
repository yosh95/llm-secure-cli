use crate::consts::{MAX_OUTPUT_CHARS, MAX_OUTPUT_LINES};

pub fn truncate_output(res_str: &str) -> String {
    let original_len = res_str.len();
    let original_lines: Vec<&str> = res_str.lines().collect();
    let original_lines_count = original_lines.len();

    if original_lines_count > MAX_OUTPUT_LINES || original_len > MAX_OUTPUT_CHARS {
        let mut truncated_lines = original_lines;
        truncated_lines.truncate(MAX_OUTPUT_LINES);
        let mut joined = truncated_lines.join("\n");
        if joined.len() > MAX_OUTPUT_CHARS {
            let mut new_len = MAX_OUTPUT_CHARS;
            while new_len > 0 && !joined.is_char_boundary(new_len) {
                new_len -= 1;
            }
            joined.truncate(new_len);
        }

        let shown_lines_count = joined.lines().count();
        let shown_chars = joined.len();

        joined.push_str(&format!(
            "\n\n... (Output truncated. Shown {} of {} lines, {} of {} chars.)",
            shown_lines_count, original_lines_count, shown_chars, original_len
        ));
        joined
    } else {
        res_str.to_string()
    }
}

pub fn truncate_json_strings(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::String(s) if s.len() > MAX_OUTPUT_CHARS => {
            *s = truncate_output(s);
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                truncate_json_strings(item);
            }
        }
        serde_json::Value::Object(obj) => {
            for (_, value) in obj {
                truncate_json_strings(value);
            }
        }
        _ => {}
    }
}
