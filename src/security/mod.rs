pub mod audit;
pub mod identity;
pub mod key_storage;
pub mod merkle;
pub mod merkle_anchor;
pub mod permissions;
pub mod policy;
pub mod pqc;
pub mod pqc_cose;

pub mod static_analyzer;
pub mod verifier;

// Re-export audit types for downstream convenience
pub use audit::{AuditEntry, AuditParams, AuditParamsBuilder, AuditStatus};

// Re-export key management abstractions for custom KeyStore implementations
pub use identity::{FileSystemKeyStore, KeyStore};

// Re-export config validation types for convenience
pub use crate::config::models::ValidationError;

/// Validates a tool call using Phase 1 security checks.
///
/// Phase 1 performs fast, deterministic checks for physical anomalies
/// (null bytes, control characters) that could destabilize the execution
/// engine or corrupt audit logs, regardless of semantic intent.
///
/// Complex intent judgment and risk assessment are delegated to
/// Phase 2 (Verifier Committee).
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
                "Phase 1 Static Analysis blocked '{name}': argument '{key}' contains \
                     control characters or null bytes."
            ));
        }
    }

    Ok(())
}
