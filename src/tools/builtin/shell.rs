use crate::cli::ui;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::process::Command;

pub fn execute_command(args: HashMap<String, Value>) -> anyhow::Result<Value> {
    let command = args
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'command' argument"))?;

    // 1. Static Analysis (Basic keyword check)
    static_analyze(command)?;

    // 2. Dual LLM Verification
    // (Note: This should ideally be called before this tool function is invoked,
    // but we can add a hook here or in the session runner.)

    // 3. Resource Limits & Execution
    // For now, simple Command execution.
    // In production, we'd use setrlimit or a sandbox.
    #[cfg(unix)]
    let output = Command::new("sh").arg("-c").arg(command).output()?;

    #[cfg(windows)]
    let output = Command::new("cmd").arg("/C").arg(command).output()?;

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
