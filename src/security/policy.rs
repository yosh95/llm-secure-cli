use crate::security::cass::{RiskLevel, CASS_ORCHESTRATOR};
use crate::security::path_validator::validate_path;
use std::collections::HashMap;

pub struct EvaluationContext {
    pub user_id: String,
    pub user_prompt: String,
    pub has_pqc_proof: bool,
}

pub struct PolicyEngine;

impl PolicyEngine {
    pub fn evaluate(
        &self,
        tool_name: &str,
        arguments: &HashMap<String, serde_json::Value>,
        context: &EvaluationContext,
    ) -> bool {
        let risk_level = CASS_ORCHESTRATOR.evaluate_risk(tool_name);
        let security_level = "high"; // Default

        // 1. Identity Requirement
        if risk_level == RiskLevel::High && !context.has_pqc_proof && security_level == "high" {
            return false;
        }

        // 2. Scope Verification
        if !self.verify_scope(tool_name, arguments) {
            return false;
        }

        // 3. Global Guardrails
        if !self.global_guardrails(tool_name, arguments) {
            return false;
        }

        true
    }

    fn verify_scope(
        &self,
        _tool_name: &str,
        arguments: &HashMap<String, serde_json::Value>,
    ) -> bool {
        let path_args = [
            "path",
            "directory",
            "file",
            "filename",
            "src",
            "dest",
            "destination",
        ];
        for arg_name in path_args {
            if let Some(raw_path) = arguments.get(arg_name).and_then(|v| v.as_str()) {
                if validate_path(raw_path).is_err() {
                    return false;
                }
            }
        }
        true
    }

    fn global_guardrails(
        &self,
        _tool_name: &str,
        arguments: &HashMap<String, serde_json::Value>,
    ) -> bool {
        let path_args = [
            "path",
            "directory",
            "file",
            "filename",
            "src",
            "dest",
            "destination",
        ];
        for arg_name in path_args {
            if let Some(raw_path) = arguments.get(arg_name).and_then(|v| v.as_str()) {
                // validate_path already handles blocked_paths
                if validate_path(raw_path).is_err() {
                    return false;
                }
            }
        }
        true
    }
}

pub static POLICY_ENGINE: PolicyEngine = PolicyEngine;
