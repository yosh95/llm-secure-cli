pub mod audit;
pub mod cass;
pub mod dual_llm_verifier;
pub mod identity;
pub mod integrity;
pub mod key_storage;
pub mod merkle;
pub mod merkle_anchor;
pub mod path_validator;
pub mod permissions;
pub mod policy;
pub mod pqc;
pub mod pqc_cose;
pub mod static_analyzer;

/// Validates a tool call using Phase 1 security checks.
///
/// Phase 1 performs fast, deterministic checks for physical anomalies
/// (null bytes, control characters) that could destabilize the execution
/// engine or corrupt audit logs, regardless of semantic intent.
///
/// Complex intent judgment and risk assessment are delegated to
/// Phase 2 (CASS) and Phase 3 (Dual LLM Verifier).
pub fn validate_tool_call(
    name: &str,
    args: &serde_json::Map<String, serde_json::Value>,
    _config: &crate::config::models::SecurityConfig,
) -> Result<(), String> {
    use crate::security::static_analyzer::StaticAnalyzer;

    // Scan every string value in args for control characters / null bytes.
    // This is a deterministic fast-fail for physical anomalies regardless of tool.
    for (key, value) in args {
        if let Some(s) = value.as_str()
            && StaticAnalyzer::is_obviously_malicious(s)
        {
            return Err(format!(
                "Phase 1 Static Analysis blocked '{}': argument '{}' contains \
                     control characters or null bytes.",
                name, key
            ));
        }
    }

    Ok(())
}
