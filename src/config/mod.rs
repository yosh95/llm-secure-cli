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
    /// Stores a `Some(error)` when the initial load from disk fails.
    /// `None` means the config was loaded successfully (or hasn't been
    /// attempted yet — the `OnceLock` itself acts as the "initialized" flag).
    config_init_error: OnceLock<Option<anyhow::Error>>,
    app_config: RwLock<Arc<AppConfig>>,
    app_state: RwLock<AppState>,
    env_cache: OnceLock<HashMap<String, String>>,
}

impl ConfigManager {
    pub fn new() -> Self {
        Self {
            config_init_error: OnceLock::new(),
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
        // The `OnceLock<Option<anyhow::Error>>` acts as a one-shot flag:
        //   - `None`    = initialized successfully (or not yet — see `get_or_init`).
        //   - `Some(e)` = initialization failed; return the same error on every call.
        //
        // Because `get_or_init` runs the closure at most once, the *load-from-disk*
        // logic is guaranteed to execute exactly once.  Afterwards the `RwLock` may
        // be overwritten by `set_config()` (e.g. `/verify on`), but the init error
        // flag stays forever — a persistent disk-load failure cannot be recovered
        // from without restarting the process.
        let init_error: &Option<anyhow::Error> = self.config_init_error.get_or_init(|| {
            match Self::load_config_from_disk() {
                Ok(config) => {
                    let arc = Arc::new(config);
                    if let Ok(mut guard) = self.app_config.write() {
                        *guard = Arc::clone(&arc);
                    } else {
                        tracing::error!("Config RwLock poisoned during initialization");
                    }
                    None // success
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to load configuration from disk");
                    // Keep the default (empty) AppConfig in the RwLock so that
                    // basic CLI commands (e.g. `llsc models`) can still work.
                    Some(e)
                }
            }
        });

        if let Some(e) = init_error {
            return Err(anyhow::anyhow!("{}", e));
        }

        let read = self
            .app_config
            .read()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        Ok(Arc::clone(&read))
    }

    /// Load and validate the config from disk.
    fn load_config_from_disk() -> anyhow::Result<AppConfig> {
        // 1. Load defaults from embedded defaults.toml
        let defaults_toml = include_str!("defaults.toml");
        let mut config_value: serde_json::Value = toml::from_str(defaults_toml)
            .map_err(|e| anyhow::anyhow!("Embedded defaults.toml must be valid: {}", e))?;

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
        let final_config_struct: AppConfig = serde_json::from_value(config_value).map_err(|e| {
            anyhow::anyhow!(
                "Failed to deserialize merged configuration: {}\n\
                     Please check your {}/config.toml for schema errors.",
                e,
                get_base_dir().to_string_lossy()
            )
        })?;

        // 4. Validate critical security settings
        if let Err(e) = validate_security_config(&final_config_struct.security) {
            return Err(anyhow::anyhow!("Invalid security configuration: {}", e));
        }

        Ok(final_config_struct)
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
        let _ = self.config_init_error.set(None);
        let mut write = self
            .app_config
            .write()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        *write = Arc::new(config);
        Ok(())
    }
}

/// Validate critical security configuration settings at load time.
/// Delegates to [`crate::config::models::SecurityConfig::validate_or_err`].
fn validate_security_config(
    security: &crate::config::models::SecurityConfig,
) -> Result<(), String> {
    security.validate_or_err()
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

impl ConfigManager {
    /// Load user-defined prompt templates from `~/.llm_secure_cli/templates/`.
    ///
    /// Only `.txt` and `.md` files are read.  The file stem (name without
    /// extension) becomes the template name.  Directory entries are created
    /// automatically if they don't exist yet.
    pub fn load_templates(&self) -> HashMap<String, String> {
        let dir = crate::consts::templates_dir();
        let mut templates = HashMap::new();

        let dir = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => {
                // Create the directory if missing
                if let Err(e) = std::fs::create_dir_all(&dir) {
                    tracing::warn!(
                        path = %dir.display(),
                        error = %e,
                        "Failed to create templates directory"
                    );
                }
                return templates;
            }
        };

        for entry in dir.filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext != "txt" && ext != "md" {
                continue;
            }

            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if name.is_empty() {
                continue;
            }

            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    templates.insert(name, content);
                }
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "Failed to read template file"
                    );
                }
            }
        }

        templates
    }
}
