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
pub mod resource_manager;
pub mod static_analyzer;

/// Validates a tool call using Phase 1 security checks (Path, Basic Sanity).
/// Returns Ok(()) if safe, or Err(message) if blocked.
pub fn validate_tool_call(
    name: &str,
    args: &serde_json::Map<String, serde_json::Value>,
    config: &crate::config::models::SecurityConfig,
) -> Result<(), String> {
    // 1. Path Guardrails (Simplified)
    let path_args = ["path", "directory", "file", "src", "dest", "filename"];
    for arg_name in path_args {
        if let Some(p_val) = args.get(arg_name).and_then(|v| v.as_str())
            && let Err(e) = crate::security::path_validator::validate_path(p_val, config)
        {
            return Err(format!("Security Blocked (Path Guardrails): {}", e));
        }
    }

    // 2. Fast-fail Syntactic Check
    if name == "execute_command" {
        let program = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
        let cmd_args: Vec<String> = args
            .get("args")
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

    // Note: Intent alignment and semantic safety are now deferred to the Dual LLM Verifier,
    // which uses the Security Context and Constitution for a holistic semantic check.

    Ok(())
}
