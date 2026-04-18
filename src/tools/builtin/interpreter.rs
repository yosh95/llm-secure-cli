use crate::config::CONFIG_MANAGER;
use crate::consts::{MAX_OUTPUT_CHARS, MAX_OUTPUT_LINES};
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

pub fn execute_python(args: HashMap<String, Value>) -> anyhow::Result<Value> {
    let code = args
        .get("code")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'code' argument"))?;

    // TODO: Static analysis (AST)

    let config = CONFIG_MANAGER.get_config();
    let timeout = config.general.command_timeout;
    let mem_limit_mb = config.general.max_command_memory_mb;
    let file_limit_mb = 100; // Default

    let tmp_dir = tempfile::Builder::new()
        .prefix("llm_secure_cli_exec_")
        .suffix("_secure")
        .tempdir()?;

    // chmod 700 on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(tmp_dir.path(), std::fs::Permissions::from_mode(0o700))?;
    }

    let script_path = tmp_dir.path().join("script.py");
    std::fs::write(&script_path, code)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o600))?;
    }

    let python_exe = if cfg!(windows) { "python" } else { "python3" };

    let mut cmd = Command::new(python_exe);
    cmd.arg(&script_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());

    // Filter environment variables
    let safe_env_keys = [
        "PATH",
        "LANG",
        "LC_ALL",
        "TERM",
        "HOME",
        "USER",
        "PWD",
        "TMPDIR",
        "TEMP",
        "TMP",
        "GIT_AUTHOR_NAME",
        "GIT_AUTHOR_EMAIL",
        "GIT_COMMITTER_NAME",
        "GIT_COMMITTER_EMAIL",
        "GIT_CONFIG_NOSYSTEM",
    ];

    cmd.env_clear();
    for key in safe_env_keys {
        if let Ok(val) = env::var(key) {
            cmd.env(key, val);
        }
    }
    // Add user-allowed env vars
    for key in &config.security.allowed_env_vars {
        if let Ok(val) = env::var(key) {
            cmd.env(key, val);
        }
    }

    #[cfg(unix)]
    unsafe {
        cmd.pre_exec(move || {
            crate::security::resource_manager::set_resource_limits(
                mem_limit_mb,
                timeout,
                file_limit_mb,
            );
            Ok(())
        });
    }

    let child = cmd.spawn()?;

    crate::security::resource_manager::limit_process_resources(&child, mem_limit_mb);

    // Timeout logic
    let result = match child.wait_with_output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let exit_code = output.status.code().unwrap_or(-1);

            let mut res_str = format!(
                "--- STDOUT ---\n{}\n",
                if stdout.is_empty() {
                    "(no output)"
                } else {
                    stdout.trim()
                }
            );
            res_str.push_str(&format!(
                "--- STDERR ---\n{}\n",
                if stderr.is_empty() {
                    "(no output)"
                } else {
                    stderr.trim()
                }
            ));
            res_str.push_str(&format!("--- EXIT CODE: {} ---", exit_code));

            // Truncate output
            let truncated = truncate_output(&res_str, MAX_OUTPUT_LINES, MAX_OUTPUT_CHARS);
            Ok(Value::String(truncated))
        }
        Err(e) => Err(anyhow::anyhow!("Execution failed: {}", e)),
    };

    result
}

fn truncate_output(s: &str, max_lines: usize, max_chars: usize) -> String {
    let mut lines: Vec<&str> = s.lines().collect();
    let mut result = s.to_string();

    if lines.len() > max_lines {
        lines.truncate(max_lines);
        result = lines.join("\n") + "\n(output truncated due to line limit)";
    }

    if result.len() > max_chars {
        result.truncate(max_chars);
        result.push_str("\n(output truncated due to character limit)");
    }

    result
}
