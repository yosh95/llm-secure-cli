use crate::config::models::SecurityConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityPosture {
    pub require_pqc_signature: bool,
    pub pqc_variant: String,
    pub require_pqc_audit_encryption: bool,
    pub ast_strictness: String,
    pub require_dual_llm_verification: bool,
}

pub struct CASSOrchestrator;

impl CASSOrchestrator {
    pub fn evaluate_risk(&self, tool_name: &str, config: &SecurityConfig) -> RiskLevel {
        let dual_llm_enabled = config.dual_llm_verification.unwrap_or(false);

        if tool_name == "execute_command" {
            if !dual_llm_enabled {
                return RiskLevel::Critical;
            } else {
                return RiskLevel::High;
            }
        }
        if config.high_risk_tools.iter().any(|t| t == tool_name) {
            RiskLevel::High
        } else if config.medium_risk_tools.iter().any(|t| t == tool_name) {
            RiskLevel::Medium
        } else {
            RiskLevel::Low
        }
    }

    pub fn get_security_requirements(
        &self,
        tool_name: &str,
        config: &SecurityConfig,
    ) -> SecurityPosture {
        let risk_level = self.evaluate_risk(tool_name, config);
        let dual_llm_enabled = config.dual_llm_verification.unwrap_or(false);

        match risk_level {
            RiskLevel::Critical => SecurityPosture {
                require_pqc_signature: true,
                pqc_variant: "ML-DSA-87".to_string(),
                require_pqc_audit_encryption: true,
                ast_strictness: "strict".to_string(),
                require_dual_llm_verification: dual_llm_enabled,
            },
            RiskLevel::High => SecurityPosture {
                require_pqc_signature: true,
                pqc_variant: "ML-DSA-87".to_string(),
                require_pqc_audit_encryption: true,
                ast_strictness: "strict".to_string(),
                require_dual_llm_verification: dual_llm_enabled,
            },
            RiskLevel::Medium => SecurityPosture {
                require_pqc_signature: true,
                pqc_variant: "ML-DSA-65".to_string(),
                require_pqc_audit_encryption: false,
                ast_strictness: "restricted".to_string(),
                require_dual_llm_verification: false,
            },
            RiskLevel::Low => SecurityPosture {
                require_pqc_signature: true,
                pqc_variant: "ML-DSA-44".to_string(),
                require_pqc_audit_encryption: false,
                ast_strictness: "basic".to_string(),
                require_dual_llm_verification: false,
            },
        }
    }
}

pub static CASS_ORCHESTRATOR: CASSOrchestrator = CASSOrchestrator;
