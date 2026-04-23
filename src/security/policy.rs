use crate::security::abac::AbacEngine;
use crate::security::cass::{CASS_ORCHESTRATOR, RiskLevel};
use crate::security::path_validator::validate_path;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct EvaluationContext {
    pub attributes: HashMap<String, Value>,
}

impl EvaluationContext {
    pub fn new() -> Self {
        let mut ctx = Self {
            attributes: HashMap::new(),
        };
        ctx.load_system_attributes();
        ctx
    }

    /// Load attributes from the system environment and execution context
    fn load_system_attributes(&mut self) {
        // Subject attributes
        let user = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());
        self.attributes
            .insert("subject.id".to_string(), Value::String(user));

        // Environment attributes: Git branch
        if let Ok(output) = std::process::Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            && output.status.success() {
                let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
                self.attributes
                    .insert("env.git_branch".to_string(), Value::String(branch));
            }

        // Environment attributes: OS
        self.attributes.insert(
            "env.os".to_string(),
            Value::String(std::env::consts::OS.to_string()),
        );

        // Environment attributes: Current Directory
        if let Ok(cwd) = std::env::current_dir() {
            self.attributes.insert(
                "env.cwd".to_string(),
                Value::String(cwd.to_string_lossy().to_string()),
            );
        }

        let keys_exist = crate::security::identity::IdentityManager::get_pqc_public_key(
            crate::security::pqc::MldsaVariant::Mldsa65,
        )
        .is_ok();

        self.attributes.insert(
            "subject.has_pqc_proof".to_string(),
            serde_json::Value::Bool(keys_exist),
        );
    }

    pub fn set_attribute(&mut self, key: &str, value: Value) {
        self.attributes.insert(key.to_string(), value);
    }

    pub fn get_attribute(&self, key: &str) -> Option<&Value> {
        self.attributes.get(key)
    }
}

pub struct PolicyEngine;

impl PolicyEngine {
    pub fn evaluate(
        &self,
        tool_name: &str,
        arguments: &serde_json::Map<String, Value>,
        context: &EvaluationContext,
    ) -> bool {
        // 1. Evaluate ABAC Rules from Config (Externalized Decision)
        if let Some(effect) = AbacEngine::evaluate(context) {
            match effect.to_lowercase().as_str() {
                "allow" => return true,
                "deny" => {
                    log::warn!("Policy Denied by ABAC rule for tool: {}", tool_name);
                    return false;
                }
                _ => {} // Continue to default checks if effect is unknown
            }
        }

        // 2. Default Hard-coded Guardrails (Safety Fallback)
        let risk_level = CASS_ORCHESTRATOR.evaluate_risk(tool_name);
        let config = crate::config::CONFIG_MANAGER.get_config();
        let security_level = std::env::var("LLM_CLI_SECURITY_LEVEL")
            .unwrap_or_else(|_| config.security.security_level.clone());

        // Identity Requirement (PQC Proof)
        let has_pqc_proof = context
            .get_attribute("subject.has_pqc_proof")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if (risk_level == RiskLevel::High || risk_level == RiskLevel::Critical)
            && !has_pqc_proof
            && security_level == "high"
        {
            log::warn!(
                "Policy Denied: Tool '{}' has high/critical risk and requires PQC proof.",
                tool_name
            );
            return false;
        }

        // Resource Guardrails (Path/Scope)
        if !self.verify_path_guardrails(tool_name, arguments) {
            return false;
        }

        true
    }

    fn verify_path_guardrails(
        &self,
        _tool_name: &str,
        arguments: &serde_json::Map<String, Value>,
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
            if let Some(raw_path) = arguments.get(arg_name).and_then(|v| v.as_str())
                && let Err(e) = validate_path(raw_path) {
                    log::warn!("Path validation failed for {}: {}", raw_path, e);
                    return false;
                }
        }
        true
    }
}

pub static POLICY_ENGINE: PolicyEngine = PolicyEngine;
