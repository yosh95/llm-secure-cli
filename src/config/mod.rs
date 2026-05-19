pub mod init;
pub mod models;

use crate::cli::ui;
use crate::config::models::{AppConfig, AppState};
use crate::consts::{config_file_path, get_base_dir, models_cache_path, state_file_path};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;
use std::sync::{Arc, OnceLock, RwLock};

/// Thread-safe configuration manager.
///
/// **`AppConfig`** is stored in a `RwLock` because it can be mutated at runtime
/// (e.g., toggling Dual LLM Verification via `/verify on|off`).  The previous
/// double-check locking pattern — acquire read lock, check, drop, acquire write
/// lock, check again — was prone to TOCTOU races.  The current implementation
/// eliminates that pattern by always acquiring the write lock for the first
/// initialization path and using a separate `OnceLock` flag to make the
/// initialization path atomic.
///
/// **`AppState`** is stored in a `RwLock` because it is mutable (*update_state*,
/// *set_alias*, …).  The lock is held only for the brief clone-or-swap, so
/// contention in the async runtime is negligible.
pub struct ConfigManager {
    /// `true` once `AppConfig` has been loaded from disk.  Coupled with a
    /// `RwLock<Arc<AppConfig>>`, this lets us avoid the old double-check
    /// locking: the `OnceLock` guarantees that the *init-from-disk* closure
    /// runs exactly once, while the `RwLock` permits later overwrites via
    /// `set_config()`.
    config_initialized: OnceLock<()>,
    app_config: RwLock<Arc<AppConfig>>,
    app_state: RwLock<AppState>,
    env_cache: OnceLock<HashMap<String, String>>,
}

impl ConfigManager {
    pub fn new() -> Self {
        Self {
            config_initialized: OnceLock::new(),
            app_config: RwLock::new(Arc::new(AppConfig::default())),
            app_state: RwLock::new(AppState::default()), // populated lazily in get_state()
            env_cache: OnceLock::new(),
        }
    }

    /// Returns a clone of the current application state.
    ///
    /// On the very first call the state is loaded from disk; subsequent calls
    /// return the in-memory copy.  Mutations (via *update_state*, etc.) are
    /// always written through to disk.
    pub fn get_state(&self) -> anyhow::Result<AppState> {
        let read = self
            .app_state
            .read()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;

        // If the state is still the default placeholder and a state file exists
        // on disk, we need to populate it.  We drop the read lock first to avoid
        // deadlocking when acquiring the write lock.
        if read.last_used_provider.is_none()
            && read.last_used_model.is_none()
            && read.last_used_v_provider.is_none()
            && read.last_used_v_model.is_none()
            && read.model_aliases.is_empty()
        {
            drop(read);
            let mut write = self
                .app_state
                .write()
                .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
            // Double-check after acquiring write lock (another thread may have
            // initialized already).
            if write.last_used_provider.is_none()
                && write.last_used_model.is_none()
                && write.model_aliases.is_empty()
            {
                *write = Self::load_state_from_disk();
            }
            return Ok(write.clone());
        }

        Ok(read.clone())
    }

    /// Load state from disk (static helper used during first access).
    fn load_state_from_disk() -> AppState {
        let s_path = state_file_path();
        if s_path.exists() {
            let content = match fs::read_to_string(&s_path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(
                        path = %s_path.display(),
                        error = %e,
                        "Failed to read state file; falling back to defaults"
                    );
                    String::new()
                }
            };
            match toml::from_str(&content) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(
                        path = %s_path.display(),
                        error = %e,
                        "Failed to parse state file; falling back to defaults"
                    );
                    AppState::default()
                }
            }
        } else {
            AppState::default()
        }
    }

    /// Helper: persist an updated state to disk, logging any write failure.
    ///
    /// IMPORTANT: This must be called **while holding the write lock** so that
    /// the in-memory state and on-disk state stay consistent.
    fn persist_state(state: &AppState) {
        if let Ok(content) = toml::to_string(state)
            && let Err(e) = fs::write(state_file_path(), content)
        {
            tracing::error!(
                path = %state_file_path().display(),
                error = %e,
                "CRITICAL: Failed to write state file — state may be lost on restart"
            );
        }
    }

    pub fn update_state(&self, provider: &str, model: &str) -> anyhow::Result<()> {
        let mut write = self
            .app_state
            .write()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        write.last_used_provider = Some(provider.to_string());
        write.last_used_model = Some(model.to_string());
        Self::persist_state(&write);
        Ok(())
    }

    pub fn update_v_state(&self, provider: &str, model: &str) -> anyhow::Result<()> {
        let mut write = self
            .app_state
            .write()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        write.last_used_v_provider = Some(provider.to_string());
        write.last_used_v_model = Some(model.to_string());
        Self::persist_state(&write);
        Ok(())
    }

    pub fn set_alias(&self, alias: &str, target: &str) -> anyhow::Result<()> {
        let mut write = self
            .app_state
            .write()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        write.model_aliases.insert(
            alias.to_string(),
            crate::config::models::ModelAlias {
                target: target.to_string(),
            },
        );
        Self::persist_state(&write);
        Ok(())
    }

    pub async fn get_cached_models(&self) -> HashMap<String, Vec<String>> {
        let c_path = models_cache_path();
        if !c_path.exists() {
            return self.update_models_cache().await;
        }

        let content = match fs::read_to_string(&c_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    path = %c_path.display(),
                    error = %e,
                    "Failed to read models cache; falling back to empty cache"
                );
                String::new()
            }
        };
        match serde_json::from_str(&content) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    path = %c_path.display(),
                    error = %e,
                    "Failed to parse models cache; falling back to empty cache"
                );
                HashMap::new()
            }
        }
    }

    pub fn get_cached_models_sync(&self) -> HashMap<String, Vec<String>> {
        let c_path = models_cache_path();
        if !c_path.exists() {
            return HashMap::new();
        }

        let content = match fs::read_to_string(&c_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    path = %c_path.display(),
                    error = %e,
                    "Failed to read models cache (sync); falling back to empty cache"
                );
                String::new()
            }
        };
        match serde_json::from_str(&content) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    path = %c_path.display(),
                    error = %e,
                    "Failed to parse models cache (sync); falling back to empty cache"
                );
                HashMap::new()
            }
        }
    }

    pub async fn update_models_cache(&self) -> HashMap<String, Vec<String>> {
        let providers = self.get_active_providers();
        let mut cache = HashMap::new();

        for p in providers {
            if let Ok(models) = self.fetch_models_from_provider(&p).await {
                cache.insert(p, models);
            }
        }

        if let Ok(content) = serde_json::to_string_pretty(&cache)
            && let Err(e) = fs::write(models_cache_path(), &content)
        {
            tracing::error!(
                path = %models_cache_path().display(),
                error = %e,
                "Failed to write models cache"
            );
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
            let dotenv_paths = [Path::new(".env").to_path_buf(), get_base_dir().join(".env")];

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
        // The `OnceLock` guarantees that the *load-from-disk* logic runs exactly
        // once, even under concurrent access.  After initialization the
        // `RwLock<Arc<AppConfig>>` is read without any TOCTOU window because
        // the OnceLock flag is already set.
        self.config_initialized.get_or_init(|| {
            let config = Self::load_config_from_disk();
            match self.app_config.write() {
                Ok(mut guard) => *guard = Arc::new(config),
                Err(e) => {
                    tracing::error!(error = %e, "Config RwLock poisoned during initialization");
                }
            }
        });

        let read = self
            .app_config
            .read()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        Ok(Arc::clone(&read))
    }

    /// Load and validate the config from disk.
    fn load_config_from_disk() -> AppConfig {
        // 1. Load defaults from embedded defaults.toml
        let defaults_toml = include_str!("defaults.toml");
        let mut config_value: serde_json::Value = match toml::from_str(defaults_toml) {
            Ok(v) => v,
            Err(e) => {
                panic!("Embedded defaults.toml must be valid: {}", e);
            }
        };

        // 2. Load user config from files and merge them
        let config_path = config_file_path();

        if config_path.exists()
            && let Ok(content) = fs::read_to_string(&config_path)
        {
            match toml::from_str::<serde_json::Value>(&content) {
                Ok(user_value) => merge_json(&mut config_value, user_value),
                Err(e) => ui::report_warning(&format!(
                    "Failed to parse config file at {:?}: {}",
                    config_path, e
                )),
            }
        }

        // 3. Final deserialization into AppConfig
        let final_config_struct: AppConfig =
            serde_json::from_value(config_value).unwrap_or_else(|e| {
                panic!(
                    "CRITICAL: Failed to deserialize merged configuration: {}\nPlease check your {}/config.toml for schema errors.",
                    e,
                    get_base_dir().to_string_lossy()
                )
            });

        // 4. Validate critical security settings
        if let Err(e) = validate_security_config(&final_config_struct.security) {
            panic!("Invalid security configuration: {}", e);
        }

        final_config_struct
    }

    /// Resolve the dual LLM provider and model, prioritizing AppState (state.toml)
    /// but falling back to AppConfig (config.toml).
    pub fn get_dual_llm_settings(&self) -> (String, String) {
        let state = self.get_state().unwrap_or_default();
        let config = self.get_config().ok();

        let provider = state
            .last_used_v_provider
            .filter(|s| !s.is_empty())
            .or_else(|| {
                config
                    .as_ref()
                    .map(|c| c.security.dual_llm_provider.clone())
            })
            .unwrap_or_default();

        let model = state
            .last_used_v_model
            .filter(|s| !s.is_empty())
            .or_else(|| config.as_ref().map(|c| c.security.dual_llm_model.clone()))
            .unwrap_or_default();

        (provider, model)
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

    /// Overwrites the in-memory application config.
    ///
    /// This is used by interactive commands (e.g., `/verify on|off`) to toggle
    /// settings at runtime.  The write lock is held only for the brief swap.
    pub fn set_config(&self, config: AppConfig) -> anyhow::Result<()> {
        // Ensure config is marked as initialized so future get_config() calls
        // don't try to reload from disk.
        let _ = self.config_initialized.set(());
        let mut write = self
            .app_config
            .write()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        *write = Arc::new(config);
        Ok(())
    }
}

/// Validate critical security configuration settings at load time.
/// Returns Ok(()) if all values are valid, or an error message describing the issue.
fn validate_security_config(
    security: &crate::config::models::SecurityConfig,
) -> Result<(), String> {
    // Validate auto_approval_level: must be one of "none", "low", "medium" (or empty, which defaults to "none")
    if let Some(ref level) = security.auto_approval_level {
        match level.as_str() {
            "none" | "low" | "medium" => {}
            other => {
                return Err(format!(
                    "auto_approval_level must be one of 'none', 'low', or 'medium', got '{}'",
                    other
                ));
            }
        }
    }

    // Validate verifier_fallback: must be "require_approval" or "block"
    match security.verifier_fallback.as_str() {
        "require_approval" | "block" => {}
        other => {
            return Err(format!(
                "verifier_fallback must be 'require_approval' or 'block', got '{}'",
                other
            ));
        }
    }

    // Validate dual_llm_confidence_threshold: must be in (0.0, 1.0]
    let threshold = security.dual_llm_confidence_threshold;
    if threshold <= 0.0 || threshold > 1.0 {
        return Err(format!(
            "dual_llm_confidence_threshold must be in range (0.0, 1.0], got {}",
            threshold
        ));
    }

    // Validate security_level: must be "high" or "standard"
    match security.security_level.as_str() {
        "high" | "standard" => {}
        other => {
            return Err(format!(
                "security_level must be 'high' or 'standard', got '{}'",
                other
            ));
        }
    }

    Ok(())
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
