pub mod audit;
pub mod cass;
pub mod dual_llm_verifier;
pub mod identity;
pub mod integrity;
pub mod merkle;
pub mod merkle_anchor;
pub mod path_validator;
pub mod permissions;
pub mod policy;
pub mod pqc;
pub mod pqc_cose;
pub mod static_analyzer;

/// Validates a tool call using Phase 1 security checks (Path, Basic Sanity).
/// Returns Ok(()) if safe, or Err(message) if blocked.
pub fn validate_tool_call(
    name: &str,
    args: &serde_json::Map<String, serde_json::Value>,
    config: &crate::config::models::SecurityConfig,
) -> Result<(), String> {
    // 1. Path Guardrails (Enhanced for MCP/Generic Tools)
    // Check any argument that looks like a path.
    let path_patterns = [
        "path",
        "directory",
        "dir",
        "file",
        "src",
        "dest",
        "filename",
        "filepath",
        "root",
    ];

    for (arg_key, arg_val) in args {
        let key_lower = arg_key.to_lowercase();
        let is_path_key = path_patterns.iter().any(|&p| key_lower.contains(p));

        let mut values_to_check = Vec::new();
        if let Some(p_arr) = arg_val.as_array() {
            for item in p_arr {
                if let Some(s) = item.as_str() {
                    values_to_check.push(s);
                }
            }
        } else if let Some(s) = arg_val.as_str() {
            values_to_check.push(s);
        }

        for s_val in values_to_check {
            // Check if it's a known path key OR the value looks like a path/URI
            let looks_like_path = s_val.contains('/')
                || s_val.contains('\\')
                || s_val.starts_with("./")
                || s_val.starts_with("../");

            if (is_path_key || looks_like_path)
                && let Err(e) = crate::security::path_validator::validate_path(s_val, config)
            {
                return Err(format!("Security Blocked (Path Guardrails): {}", e));
            }
        }
    }

    // 2. Command/Shell Validation (Generalized for MCP)
    // Detect if this is a command execution tool (local or remote MCP)
    let is_exec = name == "execute_command"
        || name.ends_with("__execute_command")
        || name.contains("run_shell")
        || name.contains("shell_execute")
        || name.contains("command_exec");

    if is_exec {
        let program = args
            .get("command")
            .or_else(|| args.get("cmd"))
            .or_else(|| args.get("executable"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let cmd_args: Vec<String> = args
            .get("args")
            .or_else(|| args.get("arguments"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let (safe, violations) =
            crate::security::static_analyzer::StaticAnalyzer::check(program, &cmd_args);
        if !safe {
            return Err(format!(
                "Security Blocked (Static Analysis): {}",
                violations.join(", ")
            ));
        }
    }

    Ok(())
}
