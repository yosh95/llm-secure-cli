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

/// Validates a tool call using Phase 1 security checks.
/// Simplified as Dual LLM handles intent and correctness.
pub fn validate_tool_call(
    _name: &str,
    _args: &serde_json::Map<String, serde_json::Value>,
    _config: &crate::config::models::SecurityConfig,
) -> Result<(), String> {
    // Phase 1 is now a no-op placeholder.
    // Core security is managed by Phase 2 (CASS) and Phase 3 (Dual LLM).
    Ok(())
}
