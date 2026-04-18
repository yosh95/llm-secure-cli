use crate::modules::models::{ContentPart, DataSource};
use crate::security::cass::{RiskLevel, SecurityPosture, CASS_ORCHESTRATOR};
use std::collections::HashMap;
use std::fmt;

pub trait AgentContext {
    fn get_input(&self, message: &str) -> String;
}

pub struct ToolExecutionContext<'a> {
    pub session: &'a dyn AgentContext,
    pub part: ContentPart,
    pub duration: Option<f64>,

    // Derived fields
    pub tool_id: String,
    pub call_id: Option<String>,
    pub name: String,
    pub args: HashMap<String, serde_json::Value>,
    pub thought_signature: Option<String>,

    // Output fields
    pub result_data: Option<serde_json::Value>,
    pub injected_data: Option<DataSource>,
    pub error_message: Option<String>,
    pub aborted: bool,
    pub security_warnings: Vec<(String, String)>,

    // Security fields
    pub risk_level: RiskLevel,
    pub security_requirements: SecurityPosture,
    pub server_name: Option<String>,
}

impl<'a> fmt::Debug for ToolExecutionContext<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolExecutionContext")
            .field("session", &"dyn AgentContext")
            .field("part", &self.part)
            .field("duration", &self.duration)
            .field("tool_id", &self.tool_id)
            .field("call_id", &self.call_id)
            .field("name", &self.name)
            .field("args", &self.args)
            .field("thought_signature", &self.thought_signature)
            .field("result_data", &self.result_data)
            .field("injected_data", &self.injected_data)
            .field("error_message", &self.error_message)
            .field("aborted", &self.aborted)
            .field("security_warnings", &self.security_warnings)
            .field("risk_level", &self.risk_level)
            .field("security_requirements", &self.security_requirements)
            .field("server_name", &self.server_name)
            .finish()
    }
}

impl<'a> ToolExecutionContext<'a> {
    pub fn new(session: &'a dyn AgentContext, part: ContentPart, duration: Option<f64>) -> Self {
        let mut tool_id = "unknown".to_string();
        let mut call_id = None;
        let mut name = "unknown".to_string();
        let mut args = HashMap::new();
        let thought_signature = part.thought_signature.clone();

        if let Some(call) = &part.function_call {
            tool_id = call
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            call_id = call
                .get("call_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            name = call
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            args = call
                .get("arguments")
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .collect();
        }

        let (server_name, base_name) = if name.contains("__") {
            let parts: Vec<&str> = name.splitn(2, "__").collect();
            (Some(parts[0].to_string()), parts[1].to_string())
        } else {
            (None, name.clone())
        };

        let risk_level = CASS_ORCHESTRATOR.evaluate_risk(&base_name);
        let security_requirements = CASS_ORCHESTRATOR.get_security_requirements(&base_name);

        Self {
            session,
            part,
            duration,
            tool_id,
            call_id,
            name,
            args,
            thought_signature,
            result_data: None,
            injected_data: None,
            error_message: None,
            aborted: false,
            security_warnings: Vec::new(),
            risk_level,
            security_requirements,
            server_name,
        }
    }
}
