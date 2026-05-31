use colored::Colorize;
use serde_json::Value;

#[derive(Clone)]
pub struct ToolResultStats {
    pub byte_count: usize,
    pub line_count: usize,
    pub item_count: Option<usize>,
    /// Label for `item_count`, e.g. "files", "matches", "items".
    pub item_label: &'static str,
    pub stderr: Option<String>,
    pub stderr_byte_count: usize,
    pub stderr_line_count: usize,
}

#[must_use]
pub fn get_tool_result_stats(name: &str, result: &Value) -> ToolResultStats {
    let mut byte_count = 0;
    let mut line_count = 0;
    let mut item_count = None;
    let mut item_label = "items";
    let mut stderr = None;
    let mut stderr_byte_count = 0;
    let mut stderr_line_count = 0;

    if let Some(s) = result.as_str() {
        byte_count = s.len();
        line_count = s.lines().count();
        // Check if it's JSON inside a string
        if let Ok(v) = serde_json::from_str::<Value>(s) {
            return get_tool_result_stats(name, &v);
        }
    } else if let Some(obj) = result.as_object() {
        // For structured data, we might want to sum up certain fields or just the whole JSON
        let serialized = result.to_string();
        byte_count = serialized.len();
        line_count = serialized.lines().count();

        // Special handling for command execution
        if let Some(stdout) = obj.get("stdout").and_then(|v| v.as_str()) {
            byte_count = stdout.len();
            line_count = stdout.lines().count();
        }
        if let Some(se) = obj.get("stderr").and_then(|v| v.as_str())
            && !se.is_empty()
        {
            stderr = Some(se.to_string());
            stderr_byte_count = se.len();
            stderr_line_count = se.lines().count();
        }

        // Special handling for item lists — choose a label appropriate to the tool
        if name == "brave_search"
            && let Some(arr) = obj.get("results").and_then(|a| a.as_array())
        {
            item_count = Some(arr.len());
            item_label = "items";
        }
        // tool_outputs often use "content"
        if let Some(content) = obj.get("content").and_then(|v| v.as_str()) {
            byte_count = content.len();
            line_count = content.lines().count();
        }
    }

    ToolResultStats {
        byte_count,
        line_count,
        item_count,
        item_label,
        stderr,
        stderr_byte_count,
        stderr_line_count,
    }
}

pub fn print_tool_stats(stats: &ToolResultStats) {
    let mut prefix = String::new();
    if stats.item_count.is_none() {
        prefix = "stdout: ".to_string();
    }
    let mut parts = vec![format!(
        "  {}{} bytes",
        prefix,
        crate::utils::format_number(stats.byte_count)
    )];
    if let Some(fc) = stats.item_count {
        parts.push(format!(
            "{} {}",
            crate::utils::format_number(fc),
            stats.item_label
        ));
    } else {
        parts.push(format!(
            "{} lines",
            crate::utils::format_number(stats.line_count)
        ));
    }

    if stats.stderr_byte_count > 0 {
        parts.push(format!(
            "stderr: {} bytes / {} lines",
            crate::utils::format_number(stats.stderr_byte_count),
            crate::utils::format_number(stats.stderr_line_count)
        ));
    }

    println!("{}", parts.join(" / ").dimmed());

    if let Some(stderr) = &stats.stderr {
        println!("  {}", "STDERR:".bold().red());
        for line in stderr.lines() {
            println!("    {}", line.red());
        }
    }
}
