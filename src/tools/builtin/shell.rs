use crate::config::models::AppConfig;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;

/// Executes a system command directly without a shell.
/// Architecturally, we rely on Phase 2 (Dual LLM) for intent verification.
pub async fn execute_command(
    args: HashMap<String, Value>,
    config: Arc<AppConfig>,
) -> anyhow::Result<Value> {
    let program = args
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'command' argument"))?;

    let cmd_args: Vec<String> = match args.get("args") {
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        _ => Vec::new(),
    };

    // Validation: Ensure the command name is NOT included in args.
    // LLMs often mistakenly include the command in args (e.g., command: "ls", args: ["ls", "-l"]).
    if let Some(first_arg) = cmd_args.first()
        && (first_arg == program || program.ends_with(&format!("/{}", first_arg)))
    {
        return Err(anyhow::anyhow!(
            "Invalid arguments: The 'args' array must NOT include the command name itself. \
                 You provided command='{}' and args={:?}. \
                 It should be command='{}' and args={:?} (only the flags and parameters).",
            program,
            cmd_args,
            program,
            &cmd_args[1..]
        ));
    }

    // 1. Static Analysis (Minimalist/Semantic focus)
    let (safe, violations) =
        crate::security::static_analyzer::StaticAnalyzer::check(program, &cmd_args);
    if !safe {
        return Err(anyhow::anyhow!(
            "Security Blocked: {}",
            violations.join(", ")
        ));
    }

    // 2. Execution with Timeout
    let timeout_secs = config.general.command_timeout;

    let mut cmd = Command::new(program);
    cmd.args(&cmd_args);

    // Structural Isolation: By using Command::new directly, we avoid shell-injection
    // vulnerabilities regardless of the operating system.
    let child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    match tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait_with_output()).await {
        Ok(Ok(output)) => Ok(json!({
            "stdout": crate::tools::executor_utils::truncate_output(&String::from_utf8_lossy(&output.stdout)),
            "stderr": crate::tools::executor_utils::truncate_output(&String::from_utf8_lossy(&output.stderr)),
            "exit_code": output.status.code().unwrap_or(-1)
        })),
        Ok(Err(e)) => Err(anyhow::anyhow!("Execution error: {}", e)),
        Err(_) => Err(anyhow::anyhow!(
            "Command timed out after {} seconds",
            timeout_secs
        )),
    }
}
