pub mod init;
pub mod models;

use crate::config::models::AppConfig;
use crate::consts::{CONFIG_FILE_PATH, LLM_CLI_BASE_DIR};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

pub struct ConfigManager {
    app_config: Mutex<Option<AppConfig>>,
    env_loaded: Mutex<bool>,
}

impl ConfigManager {
    fn new() -> Self {
        Self {
            app_config: Mutex::new(None),
            env_loaded: Mutex::new(false),
        }
    }

    fn load_env_files(&self) {
        let mut env_loaded = self.env_loaded.lock().unwrap();
        if *env_loaded {
            return;
        }

        let dotenv_paths = [
            Path::new(".env").to_path_buf(),
            LLM_CLI_BASE_DIR.join(".env"),
        ];

        for path in dotenv_paths {
            if path.exists() {
                if let Ok(content) = fs::read_to_string(path) {
                    for line in content.lines() {
                        let line = line.trim();
                        if line.is_empty() || line.starts_with('#') {
                            continue;
                        }
                        if let Some((key, val)) = line.split_once('=') {
                            let key = key.trim();
                            let val = val.trim().trim_matches(|c| c == '\'' || c == '"');
                            if !key.is_empty() && env::var(key).is_err() {
                                env::set_var(key, val);
                            }
                        }
                    }
                }
            }
        }
        *env_loaded = true;
    }

    pub fn get_config(&self) -> AppConfig {
        self.load_env_files();
        let mut app_config_lock = self.app_config.lock().unwrap();
        if let Some(config) = &*app_config_lock {
            return config.clone();
        }

        // 1. Load defaults from embedded defaults.toml
        let defaults_toml = include_str!("defaults.toml");
        let mut final_config: AppConfig = toml::from_str(defaults_toml).unwrap_or_default();

        // 2. Load user config from files (Priority: current dir > home dir)
        let config_paths = [
            (*CONFIG_FILE_PATH).clone(),
            std::path::Path::new("config.toml").to_path_buf(),
        ];

        for path in config_paths {
            if path.exists() {
                if let Ok(content) = fs::read_to_string(path) {
                    if let Ok(user_config) = toml::from_str::<AppConfig>(&content) {
                        self.merge_config(&mut final_config, user_config);
                    }
                }
            }
        }

        *app_config_lock = Some(final_config.clone());
        final_config
    }

    fn merge_config(&self, base: &mut AppConfig, over: AppConfig) {
        // Merge general
        if over.general.unified_default_provider != "google" {
            base.general.unified_default_provider = over.general.unified_default_provider;
        }
        if over.general.request_timeout != 1800 {
            base.general.request_timeout = over.general.request_timeout;
        }
        if over.general.command_timeout != 300 {
            base.general.command_timeout = over.general.command_timeout;
        }
        if over.general.max_command_memory_mb != 1024 {
            base.general.max_command_memory_mb = over.general.max_command_memory_mb;
        }
        if over.general.max_command_file_size_mb != 100 {
            base.general.max_command_file_size_mb = over.general.max_command_file_size_mb;
        }
        if over.general.max_turns != 20 {
            base.general.max_turns = over.general.max_turns;
        }
        if !over.general.pdf_as_base64 {
            base.general.pdf_as_base64 = over.general.pdf_as_base64;
        }
        if over.general.max_chat_log_lines != 10000 {
            base.general.max_chat_log_lines = over.general.max_chat_log_lines;
        }
        if over.general.max_security_log_lines != 1000 {
            base.general.max_security_log_lines = over.general.max_security_log_lines;
        }
        if over.general.max_audit_log_lines != 10000 {
            base.general.max_audit_log_lines = over.general.max_audit_log_lines;
        }
        if over.general.max_audit_archives != 10 {
            base.general.max_audit_archives = over.general.max_audit_archives;
        }
        if over.general.image_save_path != "~/Pictures/llm-secure-cli" {
            base.general.image_save_path = over.general.image_save_path;
        }

        // Merge providers
        for (name, over_p) in over.providers {
            if let Some(base_p) = base.providers.get_mut(&name) {
                if over_p.api_key.is_some() {
                    base_p.api_key = over_p.api_key;
                }
                if over_p.api_url.is_some() {
                    base_p.api_url = over_p.api_url;
                }
                if over_p.system_prompt.is_some() {
                    base_p.system_prompt = over_p.system_prompt;
                }
                if over_p.max_tokens.is_some() {
                    base_p.max_tokens = over_p.max_tokens;
                }
                for (m_name, m_val) in over_p.models {
                    base_p.models.insert(m_name, m_val);
                }
                for (k, v) in over_p.extra {
                    base_p.extra.insert(k, v);
                }
            } else {
                base.providers.insert(name, over_p);
            }
        }

        // Merge security
        if let Some(val) = over.security.dual_llm_verification {
            base.security.dual_llm_verification = Some(val);
        }
        if let Some(val) = over.security.auto_approval_level {
            base.security.auto_approval_level = Some(val);
        }
        if over.security.dual_llm_confidence_threshold != 0.7 {
            base.security.dual_llm_confidence_threshold =
                over.security.dual_llm_confidence_threshold;
        }
        if over.security.security_level != "high" {
            base.security.security_level = over.security.security_level;
        }
        if over.security.dual_llm_provider != "google" {
            base.security.dual_llm_provider = over.security.dual_llm_provider;
        }
        if over.security.dual_llm_model != "lite" {
            base.security.dual_llm_model = over.security.dual_llm_model;
        }
        if !over.security.default_roles.is_empty()
            && over.security.default_roles != vec!["user".to_string()]
        {
            base.security.default_roles = over.security.default_roles;
        }
        if over.security.default_user_id != "current_user" {
            base.security.default_user_id = over.security.default_user_id;
        }
        if !over.security.static_analysis_is_error {
            base.security.static_analysis_is_error = over.security.static_analysis_is_error;
        }
        if !over.security.scaling_patterns.is_empty() {
            base.security.scaling_patterns = over.security.scaling_patterns;
        }
        if !over.security.allowed_paths.is_empty()
            && over.security.allowed_paths != vec![".".to_string()]
        {
            base.security.allowed_paths = over.security.allowed_paths;
        }
        if !over.security.blocked_paths.is_empty() {
            base.security.blocked_paths = over.security.blocked_paths;
        }
        if !over.security.blocked_filenames.is_empty() {
            base.security.blocked_filenames = over.security.blocked_filenames;
        }
        if !over.security.high_risk_tools.is_empty() {
            base.security.high_risk_tools = over.security.high_risk_tools;
        }
        if !over.security.medium_risk_tools.is_empty() {
            base.security.medium_risk_tools = over.security.medium_risk_tools;
        }
        if !over.security.low_risk_tools.is_empty() {
            base.security.low_risk_tools = over.security.low_risk_tools;
        }
        if !over.security.allowed_env_vars.is_empty() {
            base.security.allowed_env_vars = over.security.allowed_env_vars;
        }

        if let Some(tools) = over.security.allowed_tools {
            base.security.allowed_tools = Some(tools);
        }

        // Merge MCP
        if !over.mcp_servers.is_empty() {
            base.mcp_servers = over.mcp_servers;
        }

        // Merge templates
        for (k, v) in over.templates {
            base.templates.insert(k, v);
        }
    }

    pub fn get_api_key(&self, provider: &str) -> Option<String> {
        self.load_env_files();
        let env_vars = match provider {
            "google" => vec!["GEMINI_API_KEY", "GOOGLE_API_KEY"],
            "openai" => vec!["OPENAI_API_KEY"],
            "anthropic" => vec!["ANTHROPIC_API_KEY"],
            "ollama" => vec!["OLLAMA_API_KEY"],
            "brave" => vec!["BRAVE_API_KEY", "BRAVE_SEARCH_API_KEY"],
            _ => vec![],
        };

        for var in env_vars {
            if let Ok(val) = env::var(var) {
                return Some(val);
            }
        }

        // Fallback to config
        let config = self.get_config();
        if let Some(p_cfg) = config.providers.get(provider) {
            if let Some(key) = &p_cfg.api_key {
                return Some(key.clone());
            }
        }

        // Special case for Ollama
        if provider == "ollama" {
            let config = self.get_config();
            if let Some(p_cfg) = config.providers.get("ollama") {
                let base_url = p_cfg.api_url.as_deref().unwrap_or("");
                if base_url.contains("localhost")
                    || base_url.contains("127.0.0.1")
                    || base_url.is_empty()
                {
                    return Some("local_bypass".to_string());
                }
            }
        }

        None
    }

    pub fn get_active_providers(&self) -> Vec<String> {
        let mut active = Vec::new();
        for provider in ["google", "openai", "anthropic", "ollama"] {
            if self.get_api_key(provider).is_some() {
                active.push(provider.to_string());
            }
        }
        active
    }

    pub fn get_model_config(
        &self,
        provider: &str,
        alias: &str,
    ) -> HashMap<String, serde_json::Value> {
        let config = self.get_config();
        let mut result = HashMap::new();

        if let Some(p_cfg) = config.providers.get(provider) {
            // Copy top-level provider settings (except models)
            for (k, v) in &p_cfg.extra {
                result.insert(k.clone(), v.clone());
            }
            if let Some(url) = &p_cfg.api_url {
                result.insert(
                    "api_url".to_string(),
                    serde_json::Value::String(url.clone()),
                );
            }

            // Resolve alias
            if let Some(model_entry) = p_cfg.models.get(alias) {
                match model_entry {
                    serde_json::Value::String(model_name) => {
                        result.insert(
                            "model".to_string(),
                            serde_json::Value::String(model_name.clone()),
                        );
                    }
                    serde_json::Value::Object(obj) => {
                        for (k, v) in obj {
                            result.insert(k.clone(), v.clone());
                        }
                    }
                    _ => {}
                }
            } else if alias == "default" {
                // Try to find a "default" model if not explicitly defined
                if p_cfg.models.contains_key("default") {
                    // already handled above
                } else if !p_cfg.models.is_empty() {
                    // Just take the first one? Or maybe there's a convention.
                    // For now, if no default, just use the alias itself as the model name.
                    result.insert(
                        "model".to_string(),
                        serde_json::Value::String(alias.to_string()),
                    );
                } else {
                    result.insert(
                        "model".to_string(),
                        serde_json::Value::String(alias.to_string()),
                    );
                }
            } else {
                result.insert(
                    "model".to_string(),
                    serde_json::Value::String(alias.to_string()),
                );
            }
        } else {
            result.insert(
                "model".to_string(),
                serde_json::Value::String(alias.to_string()),
            );
        }

        result
    }

    pub fn reload(&self) {
        let mut app_config_lock = self.app_config.lock().unwrap();
        *app_config_lock = None;
        let mut env_loaded = self.env_loaded.lock().unwrap();
        *env_loaded = false;
    }
}

pub static CONFIG_MANAGER: Lazy<ConfigManager> = Lazy::new(ConfigManager::new);
