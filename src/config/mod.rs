pub mod init;
pub mod models;

use crate::cli::ui;
use crate::config::models::{AppConfig, AppState};
use crate::consts::{CONFIG_FILE_PATH, LLM_CLI_BASE_DIR, MODELS_CACHE_PATH, STATE_FILE_PATH};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;
use std::sync::{Arc, OnceLock, RwLock};

pub struct ConfigManager {
    app_config: RwLock<Option<Arc<AppConfig>>>,
    app_state: RwLock<Option<AppState>>,
    env_cache: OnceLock<HashMap<String, String>>,
}

impl ConfigManager {
    pub fn new() -> Self {
        Self {
            app_config: RwLock::new(None),
            app_state: RwLock::new(None),
            env_cache: OnceLock::new(),
        }
    }

    pub fn get_state(&self) -> anyhow::Result<AppState> {
        {
            let read = self
                .app_state
                .read()
                .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
            if let Some(state) = &*read {
                return Ok(state.clone());
            }
        }

        let mut write = self
            .app_state
            .write()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        if let Some(state) = &*write {
            return Ok(state.clone());
        }

        let state = if STATE_FILE_PATH.exists() {
            let content = fs::read_to_string(&*STATE_FILE_PATH).unwrap_or_default();
            toml::from_str(&content).unwrap_or_default()
        } else {
            AppState::default()
        };

        *write = Some(state.clone());
        Ok(state)
    }

    pub fn update_state(&self, provider: &str, model: &str) -> anyhow::Result<()> {
        let mut state = self.get_state()?;
        state.last_used_provider = Some(provider.to_string());
        state.last_used_model = Some(model.to_string());

        let mut write = self
            .app_state
            .write()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        *write = Some(state.clone());

        if let Ok(content) = toml::to_string(&state) {
            let _ = fs::write(&*STATE_FILE_PATH, content);
        }
        Ok(())
    }

    pub fn update_v_state(&self, provider: &str, model: &str) -> anyhow::Result<()> {
        let mut state = self.get_state()?;
        state.last_used_v_provider = Some(provider.to_string());
        state.last_used_v_model = Some(model.to_string());

        let mut write = self
            .app_state
            .write()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        *write = Some(state.clone());

        if let Ok(content) = toml::to_string(&state) {
            let _ = fs::write(&*STATE_FILE_PATH, content);
        }
        Ok(())
    }

    pub async fn get_cached_models(&self) -> HashMap<String, Vec<String>> {
        if !MODELS_CACHE_PATH.exists() {
            return self.update_models_cache().await;
        }

        let content = fs::read_to_string(&*MODELS_CACHE_PATH).unwrap_or_default();
        serde_json::from_str(&content).unwrap_or_default()
    }

    pub fn get_cached_models_sync(&self) -> HashMap<String, Vec<String>> {
        if !MODELS_CACHE_PATH.exists() {
            return HashMap::new();
        }

        let content = fs::read_to_string(&*MODELS_CACHE_PATH).unwrap_or_default();
        serde_json::from_str(&content).unwrap_or_default()
    }

    pub async fn update_models_cache(&self) -> HashMap<String, Vec<String>> {
        let providers = self.get_active_providers();
        let mut cache = HashMap::new();

        for p in providers {
            if let Ok(models) = self.fetch_models_from_provider(&p).await {
                cache.insert(p, models);
            }
        }

        if let Ok(content) = serde_json::to_string_pretty(&cache) {
            let _ = fs::write(&*MODELS_CACHE_PATH, content);
        }
        cache
    }

    async fn fetch_models_from_provider(&self, provider: &str) -> anyhow::Result<Vec<String>> {
        let api_key = self.get_api_key(provider);
        let config = self.get_config()?;

        let mut url = String::new();
        if let Some(p_cfg) = config.providers.get(provider)
            && let Some(api_url) = &p_cfg.api_url
        {
            url = api_url.clone();
        }

        if url.is_empty() {
            match provider {
                "openai" => url = "https://api.openai.com/v1".to_string(),
                "openrouter" => url = "https://openrouter.ai/api/v1".to_string(),
                "ollama" => url = "http://localhost:11434/v1".to_string(),
                _ => return Err(anyhow::anyhow!("No API URL for provider")),
            }
        }

        let models_url = if provider == "ollama" && !url.contains("/v1") {
            format!("{}/api/tags", url.trim_end_matches('/'))
        } else if provider == "openrouter" && url == "https://openrouter.ai/api/v1" {
            // openrouter requires query param to include all modalities like video/image generation
            "https://openrouter.ai/api/v1/models?output_modalities=all".to_string()
        } else {
            format!("{}/models", url.trim_end_matches('/'))
        };

        let mut req = crate::utils::http::CLIENT.get(&models_url);
        if let Some(key) = api_key
            && key != "local_bypass"
        {
            req = req.header("Authorization", format!("Bearer {}", key));
        }

        let res = req.send().await?;
        let json: serde_json::Value = res.json().await?;

        let mut models = Vec::new();
        if provider == "ollama" && !url.contains("/v1") {
            if let Some(list) = json.get("models").and_then(|v| v.as_array()) {
                for m in list {
                    if let Some(name) = m.get("name").and_then(|v| v.as_str()) {
                        models.push(name.to_string());
                    }
                }
            }
        } else if let Some(data) = json.get("data").and_then(|v| v.as_array()) {
            for m in data {
                if let Some(id) = m.get("id").and_then(|v| v.as_str()) {
                    models.push(id.to_string());
                }
            }
        }

        Ok(models)
    }
}

impl Default for ConfigManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigManager {
    fn load_env_files(&self) -> &HashMap<String, String> {
        self.env_cache.get_or_init(|| {
            let mut cache = HashMap::new();
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
            cache
        })
    }

    pub fn get_config(&self) -> anyhow::Result<Arc<AppConfig>> {
        {
            let read = self
                .app_config
                .read()
                .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
            if let Some(config) = &*read {
                return Ok(Arc::clone(config));
            }
        }

        let mut write = self
            .app_config
            .write()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        if let Some(config) = &*write {
            return Ok(Arc::clone(config));
        }

        // 1. Load defaults from embedded defaults.toml
        let defaults_toml = include_str!("defaults.toml");
        let mut config_value: serde_json::Value = toml::from_str(defaults_toml)
            .map_err(|e| anyhow::anyhow!("Failed to parse defaults: {}", e))?;

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
                    Err(e) => ui::report_warning(&format!(
                        "Failed to parse config file at {:?}: {}",
                        path, e
                    )),
                }
            }
        }

        // 3. Final deserialization into AppConfig
        let final_config_struct: AppConfig =
            serde_json::from_value(config_value).map_err(|e| {
                anyhow::anyhow!("CRITICAL: Failed to deserialize merged configuration: {}\nPlease check your ~/.llm_secure_cli/config.toml for schema errors.", e)
            })?;

        let final_config = Arc::new(final_config_struct);
        *write = Some(Arc::clone(&final_config));
        Ok(final_config)
    }

    pub fn get_api_key(&self, provider: &str) -> Option<String> {
        // 1. Try generic provider-based env var (e.g., OPENROUTER_API_KEY, OLLAMA_API_KEY)
        let generic_env_var = format!("{}_API_KEY", provider.to_uppercase());
        if let Ok(val) = env::var(&generic_env_var) {
            return Some(val);
        }
        {
            let cache = self.load_env_files();
            if let Some(val) = cache.get(&generic_env_var) {
                return Some(val.clone());
            }
        }

        // Special case for Ollama:
        // Return "local_bypass" ONLY when the configured endpoint is actually local.
        if provider == "ollama"
            && let Ok(config) = self.get_config()
            && Self::is_local_ollama(&config)
        {
            return Some("local_bypass".to_string());
        }

        // 2. Fallback to config
        if let Ok(config) = self.get_config() {
            config
                .providers
                .get(provider)
                .and_then(|p| p.api_key.clone())
        } else {
            None
        }
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
        let config = match self.get_config() {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
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
        model: &str,
    ) -> HashMap<String, serde_json::Value> {
        let mut result: HashMap<String, serde_json::Value> = HashMap::new();
        let config = match self.get_config() {
            Ok(c) => c,
            Err(_) => return result,
        };

        if let Some(p_cfg) = config.providers.get(provider)
            && let Some(url) = &p_cfg.api_url
        {
            result.insert(
                "api_url".to_string(),
                serde_json::Value::String(url.clone()),
            );
        }

        // Global system prompt as default
        if let Some(prompt) = &config.general.system_prompt {
            result.insert(
                "system_prompt".to_string(),
                serde_json::Value::String(prompt.clone()),
            );
        }

        // Ensure "model" key is present
        if !result.contains_key("model") {
            result.insert(
                "model".to_string(),
                serde_json::Value::String(model.to_string()),
            );
        }

        result
    }

    pub fn set_config(&self, config: AppConfig) -> anyhow::Result<()> {
        let mut write = self
            .app_config
            .write()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        *write = Some(Arc::new(config));
        Ok(())
    }
}

fn merge_json(base: &mut serde_json::Value, over: serde_json::Value) {
    match (base, over) {
        (serde_json::Value::Object(base_map), serde_json::Value::Object(over_map)) => {
            for (k, v) in over_map {
                merge_json(base_map.entry(k).or_insert(serde_json::Value::Null), v);
            }
        }
        (base, serde_json::Value::Array(mut over_arr)) => {
            // Drop empty objects produced by all-commented [[section]] entries in TOML.
            over_arr.retain(|v| !matches!(v, serde_json::Value::Object(m) if m.is_empty()));
            if !over_arr.is_empty() {
                *base = serde_json::Value::Array(over_arr);
            }
        }
        (base, over) => {
            *base = over;
        }
    }
}
