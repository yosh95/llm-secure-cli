pub mod init;
pub mod models;

use crate::config::models::AppConfig;
use crate::consts::{CONFIG_FILE_PATH, LLM_CLI_BASE_DIR};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub struct ConfigManager {
    app_config: Mutex<Option<Arc<AppConfig>>>,
    env_cache: Mutex<HashMap<String, String>>,
    env_loaded: Mutex<bool>,
}

impl ConfigManager {
    pub fn new() -> Self {
        Self {
            app_config: Mutex::new(None),
            env_cache: Mutex::new(HashMap::new()),
            env_loaded: Mutex::new(bool::default()),
        }
    }
}

impl Default for ConfigManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigManager {
    fn load_env_files(&self) {
        let mut env_loaded = self.env_loaded.lock().unwrap();
        if *env_loaded {
            return;
        }

        let mut cache = self.env_cache.lock().unwrap();
        let dotenv_paths = [
            Path::new(".env").to_path_buf(),
            LLM_CLI_BASE_DIR.join(".env"),
        ];

        for path in dotenv_paths {
            if path.exists()
                && let Ok(content) = fs::read_to_string(path)
            {
                for line in content.lines() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with('#') {
                        continue;
                    }
                    if let Some((key, val)) = line.split_once('=') {
                        let key = key.trim().to_string();
                        let val = val
                            .trim()
                            .trim_matches(|c| c == '\'' || c == '"')
                            .to_string();
                        if !key.is_empty() {
                            cache.insert(key, val);
                        }
                    }
                }
            }
        }
        *env_loaded = true;
    }

    pub fn get_config(&self) -> Arc<AppConfig> {
        self.load_env_files();

        let mut app_config_lock = self.app_config.lock().unwrap();
        if let Some(config) = &*app_config_lock {
            return Arc::clone(config);
        }

        // 1. Load defaults from embedded defaults.toml
        let defaults_toml = include_str!("defaults.toml");
        let mut config_value: serde_json::Value =
            toml::from_str(defaults_toml).unwrap_or_else(|e| {
                eprintln!(
                    "CRITICAL ERROR: Failed to parse embedded defaults.toml: {}",
                    e
                );
                std::process::exit(1);
            });

        // 2. Load user config from files and merge them
        let config_paths = [
            (*CONFIG_FILE_PATH).clone(),
            std::path::Path::new("config.toml").to_path_buf(),
        ];

        for path in config_paths {
            if path.exists()
                && let Ok(content) = fs::read_to_string(&path)
            {
                match toml::from_str::<serde_json::Value>(&content) {
                    Ok(user_value) => merge_json(&mut config_value, user_value),
                    Err(e) => {
                        eprintln!("Warning: Failed to parse config file at {:?}: {}", path, e)
                    }
                }
            }
        }

        // 3. Final deserialization into AppConfig
        let final_config: AppConfig = serde_json::from_value(config_value).unwrap_or_else(|e| {
            eprintln!(
                "CRITICAL ERROR: Failed to deserialize merged configuration: {}",
                e
            );
            eprintln!("Please check your ~/.llm_secure_cli/config.toml for schema errors.");
            std::process::exit(1);
        });

        let config_arc = Arc::new(final_config);
        *app_config_lock = Some(Arc::clone(&config_arc));
        config_arc
    }

    pub fn get_api_key(&self, provider: &str) -> Option<String> {
        self.load_env_files();

        // 1. Try generic provider-based env var (e.g., OPENROUTER_API_KEY, OLLAMA_API_KEY)
        let generic_env_var = format!("{}_API_KEY", provider.to_uppercase());
        if let Ok(val) = env::var(&generic_env_var) {
            return Some(val);
        }
        {
            let cache = self.env_cache.lock().unwrap();
            if let Some(val) = cache.get(&generic_env_var) {
                return Some(val.clone());
            }
        }

        // Special case for Ollama:
        // Return "local_bypass" ONLY when the configured endpoint is actually local.
        if provider == "ollama" {
            let config = self.get_config();
            if Self::is_local_ollama(&config) {
                return Some("local_bypass".to_string());
            }
        }

        // 2. Fallback to config
        let config = self.get_config();
        config
            .providers
            .get(provider)
            .and_then(|p| p.api_key.clone())
    }

    /// Check whether an Ollama endpoint is local (no API key required).
    fn is_local_ollama(config: &AppConfig) -> bool {
        match config.providers.get("ollama") {
            Some(p_cfg) => {
                let base_url = p_cfg.api_url.as_deref().unwrap_or("");
                base_url.is_empty()
                    || base_url.contains("localhost")
                    || base_url.contains("127.0.0.1")
            }
            None => true, // No config → assume local
        }
    }

    pub fn get_active_providers(&self) -> Vec<String> {
        let config = self.get_config();
        let mut active = Vec::new();

        // Check all providers defined in config
        for provider_name in config.providers.keys() {
            if self.get_api_key(provider_name).is_some() {
                active.push(provider_name.clone());
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

    pub fn set_config(&self, config: AppConfig) {
        let mut app_config_lock = self.app_config.lock().unwrap();
        *app_config_lock = Some(Arc::new(config));
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
