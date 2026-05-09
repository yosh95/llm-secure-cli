use std::path::PathBuf;
use std::sync::OnceLock;

static BASE_DIR_INNER: OnceLock<PathBuf> = OnceLock::new();

pub fn init_base_dir(custom_path: Option<PathBuf>) {
    let base_dir = if let Some(path) = custom_path {
        path
    } else if let Some(env_base) = std::env::var_os("LLM_SECURE_CLI_BASE_DIR") {
        PathBuf::from(env_base)
    } else if let Some(home) = dirs::home_dir() {
        home.join(".llm_secure_cli")
    } else if let Some(home) = std::env::var_os("HOME")
        && !home.is_empty()
    {
        PathBuf::from(home).join(".llm_secure_cli")
    } else {
        panic!(
            "Could not find home directory. \
             Please set $HOME, $LLM_SECURE_CLI_BASE_DIR, or use --base-dir."
        );
    };
    let _ = BASE_DIR_INNER.set(base_dir);
}

pub fn get_base_dir() -> &'static PathBuf {
    BASE_DIR_INNER.get_or_init(|| {
        // Fallback initialization if init_base_dir wasn't called
        if let Some(env_base) = std::env::var_os("LLM_SECURE_CLI_BASE_DIR") {
            PathBuf::from(env_base)
        } else if let Some(home) = dirs::home_dir() {
            home.join(".llm_secure_cli")
        } else {
            PathBuf::from(".llm_secure_cli")
        }
    })
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
pub fn security_log_path() -> PathBuf {
    log_dir().join("security.log")
}
pub fn history_log_path() -> PathBuf {
    log_dir().join("history.log")
}
pub fn chat_log_path() -> PathBuf {
    log_dir().join("chat.log")
}

pub const MAX_OUTPUT_LINES: usize = 500;
pub const MAX_OUTPUT_CHARS: usize = 30000;
