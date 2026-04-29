use once_cell::sync::Lazy;
use std::path::PathBuf;

fn resolve_base_dir() -> PathBuf {
    // On Android, dirs::home_dir() returns None when $HOME is unset
    // (its fallback is intentionally disabled for "android" target_os).
    // Termux and other Android terminals *do* set $HOME, but to be safe
    // we fall back to the HOME env var directly before panicking.
    if let Some(home) = dirs::home_dir() {
        return home.join(".llm_secure_cli");
    }
    // $HOME may be available even when dirs::home_dir() returns None
    // (e.g. on Android Termux where getpwuid_r is unsupported).
    if let Some(home) = std::env::var_os("HOME")
        && !home.is_empty() {
            return PathBuf::from(home).join(".llm_secure_cli");
        }
    panic!(
        "Could not find home directory. \
         Please set the $HOME environment variable."
    );
}

pub static LLM_CLI_BASE_DIR: Lazy<PathBuf> = Lazy::new(resolve_base_dir);

pub static CONFIG_DIR: Lazy<PathBuf> = Lazy::new(|| LLM_CLI_BASE_DIR.clone());
pub static LOG_DIR: Lazy<PathBuf> = Lazy::new(|| LLM_CLI_BASE_DIR.join("logs"));
pub static KEY_DIR: Lazy<PathBuf> = Lazy::new(|| LLM_CLI_BASE_DIR.join("keys"));

pub static CONFIG_FILE_PATH: Lazy<PathBuf> = Lazy::new(|| CONFIG_DIR.join("config.toml"));
pub static AUDIT_LOG_PATH: Lazy<PathBuf> = Lazy::new(|| LOG_DIR.join("audit.jsonl"));
pub static SECURITY_LOG_PATH: Lazy<PathBuf> = Lazy::new(|| LOG_DIR.join("security.log"));
pub static HISTORY_LOG_PATH: Lazy<PathBuf> = Lazy::new(|| LOG_DIR.join("history.log"));
pub static CHAT_LOG_PATH: Lazy<PathBuf> = Lazy::new(|| LOG_DIR.join("chat.log"));
pub static TRAINING_METRICS_LOG_PATH: Lazy<PathBuf> =
    Lazy::new(|| LOG_DIR.join("training_metrics.jsonl"));

pub const UNKNOWN_TOOL_ID: &str = "unknown";

pub const MAX_OUTPUT_LINES: usize = 500;
pub const MAX_OUTPUT_CHARS: usize = 30000;
