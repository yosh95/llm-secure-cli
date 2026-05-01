use crate::consts::{MAX_OUTPUT_CHARS, MAX_OUTPUT_LINES};
use crate::llm::models::{ContentPart, MessagePart};
use crate::tools::executor_types::ToolExecutionContext;
use std::collections::HashMap;

pub fn truncate_output(res_str: &str) -> String {
    let original_len = res_str.len();
    let original_lines: Vec<&str> = res_str.lines().collect();
    let original_lines_count = original_lines.len();

    if original_lines_count > MAX_OUTPUT_LINES || original_len > MAX_OUTPUT_CHARS {
        let mut truncated_lines = original_lines;
        truncated_lines.truncate(MAX_OUTPUT_LINES);
        let mut joined = truncated_lines.join("\n");
        if joined.len() > MAX_OUTPUT_CHARS {
            joined.truncate(MAX_OUTPUT_CHARS);
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

pub fn create_error_response(ctx: &ToolExecutionContext) -> MessagePart {
    let err = ctx.error_message.as_deref().unwrap_or("Unknown error");
    let mut formatted_err = err.to_string();
    if !err.starts_with("[ERROR]")
        && !err.starts_with("[DENIED]")
        && !err.starts_with("Error:")
        && !err.starts_with("Security Error:")
    {
        formatted_err = format!("[ERROR] {}", err);
    }

    let mut fr = HashMap::new();
    fr.insert("id".to_string(), serde_json::json!(ctx.tool_id));
    if let Some(call_id) = &ctx.call_id {
        fr.insert("call_id".to_string(), serde_json::json!(call_id));
    }
    fr.insert("name".to_string(), serde_json::json!(ctx.name));
    fr.insert(
        "response".to_string(),
        serde_json::json!({"result": formatted_err}),
    );

    MessagePart::Part(Box::new(ContentPart {
        function_response: Some(fr),
        thought_signature: ctx.thought_signature.clone(),
        is_diagnostic: false,
        ..Default::default()
    }))
}

pub fn print_tool_output(res_str: &str) {
    crate::cli::ui::print_block(res_str, Some("[OK] Tool Output"), Some("bright_green"));
}
