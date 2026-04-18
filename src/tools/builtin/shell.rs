use crate::cli::ui;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::process::Command;

pub fn execute_command(args: HashMap<String, Value>) -> anyhow::Result<Value> {
    let program = args
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'command' argument"))?;

    let cmd_args: Vec<&str> = match args.get("args") {
        Some(Value::Array(arr)) => arr.iter().filter_map(|v| v.as_str()).collect(),
        _ => Vec::new(),
    };

    // 1. Static Analysis (Basic keyword check on the entire command)
    let full_command = format!("{} {}", program, cmd_args.join(" "));
    static_analyze(&full_command)?;

    // 2. Resource Limits & Execution
    // Directly execute the command without a shell to avoid injection and quoting issues.
    let output = Command::new(program)
        .args(&cmd_args)
        .output()?;

    Ok(json!({
        "stdout": String::from_utf8_lossy(&output.stdout),
        "stderr": String::from_utf8_lossy(&output.stderr),
        "exit_code": output.status.code().unwrap_or(-1)
    }))
}

fn static_analyze(command: &str) -> anyhow::Result<()> {
    let dangerous_patterns = [
        "rm -rf /",
        "mkfs",
        "dd if=",
        "> /etc/",
        "chmod -R 777",
        "chown",
        "passwd",
        "kill -9",
    ];

    for pattern in dangerous_patterns {
        if command.contains(pattern) {
            ui::report_warning(&format!("Dangerous command pattern detected: {}", pattern));
            // We still allow it to go to HITL, but we've warned.
            // Or we could return an error. Let's return a warning for now.
        }
    }

    Ok(())
}
