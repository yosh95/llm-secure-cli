pub mod cache;
pub mod init;
pub mod models;
pub mod state;

use crate::cli::ui;
use crate::config::models::{AppConfig, AppState};
use crate::consts::{config_file_path, get_base_dir};
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
///
/// State management methods live in [`state`]; model-cache methods in [`cache`].
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
