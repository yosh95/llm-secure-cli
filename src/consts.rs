use std::path::PathBuf;
use std::sync::OnceLock;

static BASE_DIR_INNER: OnceLock<PathBuf> = OnceLock::new();

pub fn init_base_dir(custom_path: Option<PathBuf>) {
    let base_dir = if let Some(path) = custom_path {
        path
    } else {
        default_base_dir()
    };
    if let Err(existing) = BASE_DIR_INNER.set(base_dir)
        && BASE_DIR_INNER.get() != Some(&existing)
    {
        // This should only happen if init is called twice with different paths
        tracing::warn!("Attempted to re-initialize base_dir to a different path.");
    }
}

pub fn get_base_dir() -> &'static PathBuf {
    BASE_DIR_INNER.get_or_init(default_base_dir)
}

fn default_base_dir() -> PathBuf {
    if let Some(home) = dirs::home_dir() {
        home.join(".llm_secure_cli")
    } else {
        PathBuf::from(".llm_secure_cli")
    }
}

pub static LLM_CLI_BASE_DIR: &OnceLock<PathBuf> = &BASE_DIR_INNER;

pub fn config_dir() -> PathBuf {
    get_base_dir().clone()
}
pub fn log_dir() -> PathBuf {
    get_base_dir().join("logs")
}
pub fn key_dir() -> PathBuf {
    get_base_dir().join("keys")
}

pub fn config_file_path() -> PathBuf {
    config_dir().join("config.toml")
}
pub fn state_file_path() -> PathBuf {
    config_dir().join("state.toml")
}
pub fn models_cache_path() -> PathBuf {
    config_dir().join("models_cache.json")
}
pub fn audit_log_path() -> PathBuf {
    log_dir().join("audit.jsonl")
}
/// Head-pointer cache file for O(1) lookup of the last audit entry hash.
/// Avoids scanning the entire audit log on every session start.
///
/// File format: a single line containing the SHA-256 hex hash of the last
/// audit entry, followed by a newline.  If the cache is stale or corrupt,
/// `get_last_log_hash()` falls back to a full-file scan.
pub fn audit_head_cache_path() -> PathBuf {
    log_dir().join("audit_head.cache")
}
pub fn security_log_path() -> PathBuf {
    log_dir().join("security.log")
}
pub fn history_log_path() -> PathBuf {
    log_dir().join("history.log")
}
pub fn chat_log_path() -> PathBuf {
    log_dir().join("chat.log")
}
pub fn sessions_dir() -> PathBuf {
    get_base_dir().join("sessions")
}
pub fn templates_dir() -> PathBuf {
    get_base_dir().join("templates")
}
