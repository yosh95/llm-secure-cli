#![allow(dead_code)]

use dirs::home_dir;
use once_cell::sync::Lazy;
use std::path::PathBuf;

pub static LLM_CLI_BASE_DIR: Lazy<PathBuf> = Lazy::new(|| {
    home_dir()
        .expect("Could not find home directory")
        .join(".llm_secure_cli")
});

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
