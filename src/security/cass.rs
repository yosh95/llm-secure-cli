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
    pub fn evaluate_risk(
        &self,
        tool_name: &str,
        args: Option<&serde_json::Value>,
        config: &SecurityConfig,
    ) -> RiskLevel {
        // 1. Baseline risk by tool definition (Static classification)
        let mut level = if tool_name == "execute_python"
            || config.high_risk_tools.iter().any(|t| t == tool_name)
        {
            RiskLevel::High
        } else if config.medium_risk_tools.iter().any(|t| t == tool_name) {
            RiskLevel::Medium
        } else {
            RiskLevel::Low
        };

        // 2. Dynamic escalation based on argument context
        if let Some(args_val) = args {
            let args_str = args_val.to_string().to_lowercase();

            let is_sensitive = config
                .scaling_patterns
                .iter()
                .any(|p| args_str.contains(&p.to_lowercase()))
                || config
                    .blocked_paths
                    .iter()
                    .any(|p| args_str.contains(&p.to_lowercase()));

            if is_sensitive && level < RiskLevel::High {
                level = RiskLevel::High;
            }
        }

        // 3. Environment-based risk escalation
        if config.security_level == "high" && level == RiskLevel::Low {
            level = RiskLevel::Medium;
        }

        // 4. Critical transition: High-risk tool WITHOUT dual-LLM verification
        let dual_llm_enabled = config.dual_llm_verification.unwrap_or(false);
        if level >= RiskLevel::High && !dual_llm_enabled {
            level = RiskLevel::Critical;
        }

        level
    }

    pub fn get_security_requirements(
        &self,
        tool_name: &str,
        args: Option<&serde_json::Value>,
        config: &SecurityConfig,
    ) -> SecurityPosture {
        let risk_level = self.evaluate_risk(tool_name, args, config);

        match risk_level {
            RiskLevel::Critical => SecurityPosture {
                require_pqc_signature: true,
                pqc_variant: "ML-DSA-87".to_string(), // NIST Level 5
                require_pqc_audit_encryption: true,
                ast_strictness: "strict".to_string(),
                require_dual_llm_verification: true,
            },
            RiskLevel::High => SecurityPosture {
                require_pqc_signature: true,
                pqc_variant: "ML-DSA-87".to_string(),
                require_pqc_audit_encryption: true,
                ast_strictness: "strict".to_string(),
                require_dual_llm_verification: config.dual_llm_verification.unwrap_or(false),
            },
            RiskLevel::Medium => SecurityPosture {
                require_pqc_signature: true,
                pqc_variant: "ML-DSA-65".to_string(), // NIST Level 3
                require_pqc_audit_encryption: false,
                ast_strictness: "standard".to_string(),
                require_dual_llm_verification: false,
            },
            RiskLevel::Low => SecurityPosture {
                require_pqc_signature: true,
                pqc_variant: "ML-DSA-44".to_string(), // NIST Level 2
                require_pqc_audit_encryption: false,
                ast_strictness: "relaxed".to_string(),
                require_dual_llm_verification: false,
            },
        }
    }
}

pub static CASS_ORCHESTRATOR: CASSOrchestrator = CASSOrchestrator;
