pub mod cache;
pub mod defaults;
pub mod models;
pub mod state;

use crate::config::models::{AppConfig, AppState, CliOverrides};
use crate::consts::get_base_dir;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;
use std::sync::{Arc, OnceLock, RwLock};

/// Thread-safe configuration manager.
///
/// **`AppConfig`** is stored in a `RwLock` because it can be mutated at runtime
/// (e.g., toggling Verifier via `/verify on|off`).  All configuration comes
/// from compile-time defaults (in [`defaults`]) and CLI argument overrides —
/// there is **no `config.toml` file**.
///
/// **`AppState`** is stored in a `RwLock` because it is mutable (*`update_state`*,
/// …).  The lock is held only for the brief clone-or-swap, so
/// contention in the async runtime is negligible.
///
/// State management methods live in [`state`]; model-cache methods in [`cache`].
pub struct ConfigManager {
    /// Stores a `Some(error)` when initialisation fails.
    /// `None` means the config was loaded successfully.
    config_init_error: OnceLock<Option<anyhow::Error>>,
    app_config: RwLock<Arc<AppConfig>>,
    app_state: RwLock<AppState>,
    env_cache: OnceLock<HashMap<String, String>>,
    /// CLI-provided overrides applied on top of the default config.
    cli_overrides: OnceLock<CliOverrides>,
}

impl ConfigManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            config_init_error: OnceLock::new(),
            app_config: RwLock::new(Arc::new(AppConfig::default())),
            app_state: RwLock::new(AppState::default()),
            env_cache: OnceLock::new(),
            cli_overrides: OnceLock::new(),
        }
    }

    /// Store CLI argument overrides before the config is loaded.
    /// Must be called before the first `get_config()` invocation.
    pub fn set_cli_overrides(&self, overrides: CliOverrides) {
        let _ = self.cli_overrides.set(overrides);
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
        let init_error: &Option<anyhow::Error> = self.config_init_error.get_or_init(|| {
            // Start with compiled-in defaults
            let mut config = AppConfig::default();

            // Apply CLI overrides if provided
            if let Some(overrides) = self.cli_overrides.get().cloned() {
                config = overrides.apply_to(config);
            }

            let arc = Arc::new(config);
            if let Ok(mut guard) = self.app_config.write() {
                *guard = Arc::clone(&arc);
            } else {
                tracing::error!("Config RwLock poisoned during initialization");
            }
            None // success
        });

        if let Some(e) = init_error {
            return Err(anyhow::anyhow!("{e}"));
        }

        let read = self
            .app_config
            .read()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {e}"))?;
        Ok(Arc::clone(&read))
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

        // 2. Fallback to config (default providers have no api_key set)
        if let Ok(config) = self.get_config() {
            config
                .providers
                .get(provider)
                .and_then(|p| p.api_key.clone())
        } else {
            None
        }
    }

    pub fn get_active_providers(&self) -> Vec<String> {
        let config = match self.get_config() {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        let mut active = Vec::new();

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

        if let Some(prompt) = &config.general.system_prompt {
            result.insert(
                "system_prompt".to_string(),
                serde_json::Value::String(prompt.clone()),
            );
        }

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
        if self.config_init_error.set(None).is_err() {
            tracing::warn!("config_init_error already set");
        }

        let mut write = self
            .app_config
            .write()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {e}"))?;
        *write = Arc::new(config);
        Ok(())
    }
}
