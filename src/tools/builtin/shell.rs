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
        Some(other) => {
            return Err(anyhow::anyhow!(
                "Invalid type for 'args': expected an array of strings, got {}. \
                 If you have a single argument, wrap it in an array: \
                 e.g. {{\"args\": [\"{}\"]}} instead of {{\"args\": \"{}\"}}.",
                other,
                match other {
                    Value::String(s) => s.clone(),
                    v => v.to_string(),
                },
                match other {
                    Value::String(s) => s.clone(),
                    v => v.to_string(),
                },
            ));
        }
        None => Vec::new(),
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

    // Validation: Detect shell metacharacters / operators in args.
    // This tool executes commands directly (no shell), so shell syntax like
    // redirections or pipes is silently passed as literal arguments, causing
    // confusing errors. LLMs cannot easily diagnose the root cause from those
    // downstream errors, so we fail fast with a clear explanation here.
    let shell_patterns: &[(&str, &str)] = &[
        ("2>&1", "redirect stderr to stdout"),
        ("1>&2", "redirect stdout to stderr"),
        ("2>", "redirect stderr to file"),
        (">>", "append output to file"),
        (">", "redirect output to file"),
        ("<", "redirect input from file"),
        ("|", "pipe output to another command"),
        ("&&", "run next command on success"),
        ("||", "run next command on failure"),
        (";", "command separator"),
        ("&", "run in background (shell operator)"),
    ];

    for arg in &cmd_args {
        for (pattern, description) in shell_patterns {
            if arg.contains(pattern) {
                return Err(anyhow::anyhow!(
                    "Shell operator '{}' ({}) found in args: '{}'. \
                     This tool executes commands directly without a shell, so shell operators \
                     have no effect and are passed as literal arguments. \
                     Remove '{}' from args. \
                     Note: stdout and stderr are already captured separately in the result, \
                     so redirection is not needed.",
                    pattern,
                    description,
                    arg,
                    pattern,
                ));
            }
        }
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

    let cwd = args.get("cwd").and_then(|v| v.as_str());

    // 2. Execution with Timeout
    let timeout_secs = config.general.command_timeout;

    let mut cmd = Command::new(program);
    cmd.args(&cmd_args);
    if let Some(c) = cwd {
        cmd.current_dir(c);
    }

    // Structural Isolation: By using Command::new directly, we avoid shell-injection
    // vulnerabilities regardless of the operating system.
    let child = match cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(anyhow::anyhow!(
                "Command '{}' not found in system PATH. Please ensure the command is correct and installed. Do NOT include natural language or explanations in the command field.",
                program
            ));
        }
        Err(e) => return Err(anyhow::anyhow!("Failed to start process: {}", e)),
    };

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
