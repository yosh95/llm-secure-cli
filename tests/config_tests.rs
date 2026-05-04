use llm_secure_cli::config::ConfigManager;
use std::env;
use std::fs;
use std::sync::Mutex;
use tempfile::tempdir;

static CONFIG_TEST_MUTEX: Mutex<()> = Mutex::new(());

#[test]
fn test_config_merging_from_current_dir() {
    let _guard = CONFIG_TEST_MUTEX.lock().expect("Lock poisoned");
    let tmp_dir = tempdir().expect("Failed to create temp dir");
    let original_cwd = env::current_dir().expect("Failed to get CWD");

    env::set_current_dir(tmp_dir.path()).expect("Failed to change CWD");

    // ConfigManager checks for "config.toml" in the current working directory.
    let custom_config = r#"
[security]
auto_approval_level = "low"
"#;
    fs::write("config.toml", custom_config).expect("Failed to write mock config");

    let manager = ConfigManager::new();
    let config = manager.get_config().expect("Failed to load config");

    // Default is "none", so "low" indicates it was merged from our file.
    assert_eq!(config.security.auto_approval_level.as_deref(), Some("low"));

    env::set_current_dir(original_cwd).expect("Failed to restore CWD");
}

#[test]
fn test_env_file_parsing_logic() {
    let _guard = CONFIG_TEST_MUTEX.lock().expect("Lock poisoned");
    let tmp_dir = tempdir().expect("Failed to create temp dir");
    let original_cwd = env::current_dir().expect("Failed to get CWD");
    env::set_current_dir(tmp_dir.path()).expect("Failed to change CWD");

    // .env file in CWD should be picked up by load_env_files
    let env_content = "TEST_PROVIDER_API_KEY=local_key_123\n";
    fs::write(".env", env_content).expect("Failed to write .env");

    let manager = ConfigManager::new();
    let key = manager.get_api_key("test_provider");

    assert_eq!(key, Some("local_key_123".to_string()));

    env::set_current_dir(original_cwd).expect("Failed to restore CWD");
}
