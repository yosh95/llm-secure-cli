use std::path::PathBuf;
use std::sync::LazyLock;

fn resolve_base_dir() -> PathBuf {
    if let Some(home) = dirs::home_dir() {
        return home.join(".llm_secure_cli");
    }
    // $HOME may be available even when dirs::home_dir() returns None
    if let Some(home) = std::env::var_os("HOME")
        && !home.is_empty()
    {
        return PathBuf::from(home).join(".llm_secure_cli");
    }
    panic!(
        "Could not find home directory. \
         Please set the $HOME environment variable."
    );
}

pub static LLM_CLI_BASE_DIR: LazyLock<PathBuf> = LazyLock::new(resolve_base_dir);

pub static CONFIG_DIR: LazyLock<PathBuf> = LazyLock::new(|| LLM_CLI_BASE_DIR.clone());
pub static LOG_DIR: LazyLock<PathBuf> = LazyLock::new(|| LLM_CLI_BASE_DIR.join("logs"));
pub static KEY_DIR: LazyLock<PathBuf> = LazyLock::new(|| LLM_CLI_BASE_DIR.join("keys"));

pub static CONFIG_FILE_PATH: LazyLock<PathBuf> = LazyLock::new(|| CONFIG_DIR.join("config.toml"));
pub static STATE_FILE_PATH: LazyLock<PathBuf> = LazyLock::new(|| CONFIG_DIR.join("state.toml"));
pub static MODELS_CACHE_PATH: LazyLock<PathBuf> =
    LazyLock::new(|| CONFIG_DIR.join("models_cache.json"));
pub static AUDIT_LOG_PATH: LazyLock<PathBuf> = LazyLock::new(|| LOG_DIR.join("audit.jsonl"));
pub static SECURITY_LOG_PATH: LazyLock<PathBuf> = LazyLock::new(|| LOG_DIR.join("security.log"));
pub static HISTORY_LOG_PATH: LazyLock<PathBuf> = LazyLock::new(|| LOG_DIR.join("history.log"));
pub static CHAT_LOG_PATH: LazyLock<PathBuf> = LazyLock::new(|| LOG_DIR.join("chat.log"));

pub const MAX_OUTPUT_LINES: usize = 500;
pub const MAX_OUTPUT_CHARS: usize = 30000;
