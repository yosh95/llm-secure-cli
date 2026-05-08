use colored::*;
use serde_json::Value;

#[derive(Clone)]
pub struct ToolResultStats {
    pub byte_count: usize,
    pub line_count: usize,
    pub file_count: Option<usize>,
    pub stderr: Option<String>,
}

pub fn get_tool_result_stats(result: &Value) -> ToolResultStats {
    let mut byte_count = 0;
    let mut line_count = 0;
    let mut file_count = None;
    let mut stderr = None;

    if let Some(s) = result.as_str() {
        byte_count = s.len();
        line_count = s.lines().count();
        // Check if it's JSON inside a string
        if let Ok(v) = serde_json::from_str::<Value>(s) {
            return get_tool_result_stats(&v);
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
        }

        // Special handling for file lists
        for key in ["matches", "results", "files"] {
            if let Some(arr) = obj.get(key).and_then(|a| a.as_array()) {
                file_count = Some(arr.len());
                break;
            }
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
        file_count,
        stderr,
    }
}

pub fn print_tool_stats(stats: &ToolResultStats) {
    let mut parts = vec![format!("  {} bytes", stats.byte_count)];
    if let Some(fc) = stats.file_count {
        parts.push(format!("{} files", fc));
    } else {
        parts.push(format!("{} lines", stats.line_count));
    }

    println!("{}", parts.join(" / ").bright_black());

    if let Some(stderr) = &stats.stderr {
        println!("  {}", "STDERR:".bold().red());
        for line in stderr.lines() {
            println!("    {}", line.red());
        }
    }
}
