use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GeneralConfig {
    #[serde(default = "default_unified_provider")]
    pub unified_default_provider: String,
    #[serde(default = "default_true")]
    pub pdf_as_base64: bool,
    #[serde(default = "default_request_timeout")]
    pub request_timeout: u64,
    #[serde(default = "default_command_timeout")]
    pub command_timeout: u64,
    #[serde(default = "default_max_memory")]
    pub max_command_memory_mb: u64,
    #[serde(default = "default_max_chat_log")]
    pub max_chat_log_lines: usize,
    #[serde(default = "default_max_security_log")]
    pub max_security_log_lines: usize,
    #[serde(default = "default_max_audit_log")]
    pub max_audit_log_lines: usize,
    #[serde(default = "default_max_audit_archives")]
    pub max_audit_archives: usize,
    #[serde(default = "default_image_save_path")]
    pub image_save_path: String,
}

fn default_unified_provider() -> String {
    "google".to_string()
}
fn default_true() -> bool {
    true
}
fn default_request_timeout() -> u64 {
    1800
}
fn default_command_timeout() -> u64 {
    300
}
fn default_max_memory() -> u64 {
    1024
}
fn default_max_chat_log() -> usize {
    10000
}
fn default_max_security_log() -> usize {
    1000
}
fn default_max_audit_log() -> usize {
    10000
}
fn default_max_audit_archives() -> usize {
    10
}
fn default_image_save_path() -> String {
    "~/Pictures/llm-secure-cli".to_string()
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            unified_default_provider: default_unified_provider(),
            pdf_as_base64: default_true(),
            request_timeout: default_request_timeout(),
            command_timeout: default_command_timeout(),
            max_command_memory_mb: default_max_memory(),
            max_chat_log_lines: default_max_chat_log(),
            max_security_log_lines: default_max_security_log(),
            max_audit_log_lines: default_max_audit_log(),
            max_audit_archives: default_max_audit_archives(),
            image_save_path: default_image_save_path(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ProviderConfig {
    pub api_key: Option<String>,
    pub api_url: Option<String>,
    pub system_prompt: Option<String>,
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub models: HashMap<String, serde_json::Value>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SecurityConfig {
    #[serde(default = "default_roles")]
    pub default_roles: Vec<String>,
    #[serde(default = "default_user_id")]
    pub default_user_id: String,
    #[serde(default = "default_allowed_paths")]
    pub allowed_paths: Vec<String>,
    #[serde(default)]
    pub blocked_paths: Vec<String>,
    #[serde(default)]
    pub blocked_filenames: Vec<String>,
    #[serde(default)]
    pub high_risk_tools: Vec<String>,
    #[serde(default)]
    pub medium_risk_tools: Vec<String>,
    #[serde(default)]
    pub low_risk_tools: Vec<String>,
    #[serde(default)]
    pub allowed_env_vars: Vec<String>,
    #[serde(default = "default_true")]
    pub static_analysis_is_error: bool,
    #[serde(default)]
    pub scaling_patterns: Vec<String>,
    #[serde(default)]
    pub auto_approval_level: Option<String>,
    #[serde(default)]
    pub dual_llm_verification: Option<bool>,
    #[serde(default = "default_unified_provider")]
    pub dual_llm_provider: String,
    #[serde(default = "default_dual_llm_model")]
    pub dual_llm_model: String,
}

fn default_roles() -> Vec<String> {
    vec!["user".to_string()]
}
fn default_user_id() -> String {
    "current_user".to_string()
}
fn default_allowed_paths() -> Vec<String> {
    vec![".".to_string()]
}
fn default_dual_llm_model() -> String {
    "lite".to_string()
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            default_roles: default_roles(),
            default_user_id: default_user_id(),
            allowed_paths: default_allowed_paths(),
            blocked_paths: Vec::new(),
            blocked_filenames: Vec::new(),
            high_risk_tools: Vec::new(),
            medium_risk_tools: Vec::new(),
            low_risk_tools: Vec::new(),
            allowed_env_vars: Vec::new(),
            static_analysis_is_error: true,
            scaling_patterns: Vec::new(),
            auto_approval_level: None,
            dual_llm_verification: None,
            dual_llm_provider: "google".to_string(),
            dual_llm_model: "lite".to_string(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub zero_trust: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub security: SecurityConfig,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
    #[serde(flatten)]
    pub providers: HashMap<String, ProviderConfig>,
    #[serde(default)]
    pub templates: HashMap<String, String>,
}
