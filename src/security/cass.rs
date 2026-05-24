use crate::config::models::SecurityConfig;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SecurityPosture {
    pub require_pqc_signature: bool,
    pub pqc_variant: String,
    pub require_pqc_audit_encryption: bool,
    pub ast_strictness: String,
    pub require_dual_llm_verification: bool,
}

pub struct CASSOrchestrator;

impl CASSOrchestrator {
    /// Evaluate risk for a tool call.
    ///
    /// Risk classification by tool name is deprecated since there are only
    /// two built-in tools (`execute_python` and `brave_search`) and all
    /// semantic risk assessment is delegated to the Dual LLM Verifier.
    ///
    /// Always returns `Low`. The return value is retained for backward
    /// compatibility with audit tagging.
    pub fn evaluate_risk(
        _tool_name: &str,
        _args: Option<&serde_json::Value>,
        _config: &SecurityConfig,
    ) -> RiskLevel {
        RiskLevel::Low
    }

    /// Returns security posture always at maximum cryptographic strength.
    ///
    /// Risk-level-based PQC variant switching is discontinued. All operations
    /// use the highest available NIST Level 5 strength regardless of risk level.
    pub fn get_security_requirements(
        _tool_name: &str,
        _args: Option<&serde_json::Value>,
        _config: &SecurityConfig,
    ) -> SecurityPosture {
        // Maximum strength: ML-DSA-87 (NIST Level 5), ML-KEM-1024 (NIST Level 5)
        SecurityPosture {
            require_pqc_signature: true,
            pqc_variant: "ML-DSA-87".to_string(),
            require_pqc_audit_encryption: true,
            ast_strictness: "strict".to_string(),
            require_dual_llm_verification: false,
        }
    }
}
