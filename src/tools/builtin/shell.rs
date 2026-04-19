use crate::cli::ui;
use crate::config::CONFIG_MANAGER;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;
use tokio::process::Command;

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

    // 2. Execution with Timeout
    let config = CONFIG_MANAGER.get_config();
    let timeout_secs = config.general.command_timeout;

    let rt = tokio::runtime::Handle::current();
    let output_res = tokio::task::block_in_place(|| {
        rt.block_on(async {
            let mut cmd = Command::new(program);
            cmd.args(&cmd_args);

            let child = cmd
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()?;

            match tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait_with_output())
                .await
            {
                Ok(Ok(output)) => Ok(output),
                Ok(Err(e)) => Err(anyhow::anyhow!("Execution error: {}", e)),
                Err(_) => Err(anyhow::anyhow!(
                    "Command timed out after {} seconds",
                    timeout_secs
                )),
            }
        })
    })?;

    Ok(json!({
        "stdout": String::from_utf8_lossy(&output_res.stdout),
        "stderr": String::from_utf8_lossy(&output_res.stderr),
        "exit_code": output_res.status.code().unwrap_or(-1)
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
            ui::report_error(&format!(
                "Security Blocked: Dangerous command pattern detected: {}",
                pattern
            ));
            return Err(anyhow::anyhow!(
                "Security policy violation: dangerous pattern '{}'",
                pattern
            ));
        }
    }

    Ok(())
}
