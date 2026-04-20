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
        let mut config_value: serde_json::Value =
            toml::from_str(defaults_toml).expect("Failed to parse embedded defaults.toml");

        // 2. Load user config from files and merge them
        let config_paths = [
            (*CONFIG_FILE_PATH).clone(),
            std::path::Path::new("config.toml").to_path_buf(),
        ];

        for path in config_paths {
            if path.exists() {
                if let Ok(content) = fs::read_to_string(path) {
                    if let Ok(user_value) = toml::from_str::<serde_json::Value>(&content) {
                        merge_json(&mut config_value, user_value);
                    }
                }
            }
        }

        // 3. Final deserialization into AppConfig
        let final_config: AppConfig = serde_json::from_value(config_value)
            .expect("Failed to deserialize merged configuration");

        *app_config_lock = Some(final_config.clone());
        final_config
    }

    pub fn get_api_key(&self, provider: &str) -> Option<String> {
        self.load_env_files();

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
        let mut result: HashMap<String, serde_json::Value> = HashMap::new();

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
            } else {
                // If no specific config for this alias, just use the alias itself as the model name
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

fn merge_json(base: &mut serde_json::Value, over: serde_json::Value) {
    match (base, over) {
        (serde_json::Value::Object(base_map), serde_json::Value::Object(over_map)) => {
            for (k, v) in over_map {
                merge_json(base_map.entry(k).or_insert(serde_json::Value::Null), v);
            }
        }
        (base, over) => {
            *base = over;
        }
    }
}

pub static CONFIG_MANAGER: Lazy<ConfigManager> = Lazy::new(ConfigManager::new);
