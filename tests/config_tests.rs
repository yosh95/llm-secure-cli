use llm_secure_cli::config::ConfigManager;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use tempfile::tempdir;

static CONFIG_TEST_MUTEX: Mutex<()> = Mutex::new(());
static TEST_BASE_DIR: OnceLock<PathBuf> = OnceLock::new();

fn setup_base_dir() -> PathBuf {
    TEST_BASE_DIR
        .get_or_init(|| {
            let tmp = tempdir().expect("Failed to create temp dir");
            let path = tmp.keep(); // Leak the path so it persists during the test run
            llm_secure_cli::consts::init_base_dir(Some(path.clone()));
            path
        })
        .clone()
}

#[test]
fn test_config_merging_from_current_dir() {
    let _guard = CONFIG_TEST_MUTEX.lock().expect("Lock poisoned");
    let actual_base_dir = setup_base_dir();

    let original_cwd = env::current_dir().expect("Failed to get CWD");
    let tmp_dir = tempdir().expect("Failed to create temp dir");
    env::set_current_dir(tmp_dir.path()).expect("Failed to change CWD");

    // Write config to the actual base directory where ConfigManager will look.
    let custom_config = r#"
[security]
auto_approval_level = "low"
"#;
    let config_path = actual_base_dir.join("config.toml");
    fs::write(&config_path, custom_config).expect("Failed to write mock config");

    let manager = ConfigManager::new();
    let config = manager.get_config().expect("Failed to load config");

    // Default is "none", so "low" indicates it was merged from our file.
    use llm_secure_cli::config::models::AutoApprovalLevel;
    assert_eq!(
        config.security.auto_approval_level,
        Some(AutoApprovalLevel::Low)
    );

    // Clean up
    let _ = fs::remove_file(config_path);
    env::set_current_dir(original_cwd).expect("Failed to restore CWD");
}

#[test]
fn test_env_file_parsing_logic() {
    let _guard = CONFIG_TEST_MUTEX.lock().expect("Lock poisoned");
    let actual_base_dir = setup_base_dir();

    let original_cwd = env::current_dir().expect("Failed to get CWD");
    let tmp_dir = tempdir().expect("Failed to create temp dir");
    env::set_current_dir(tmp_dir.path()).expect("Failed to change CWD");

    // .env file in CWD should be picked up by load_env_files
    let env_content = "TEST_PROVIDER_API_KEY=local_key_123\n";
    fs::write(".env", env_content).expect("Failed to write .env");

    // Also write to base dir just in case (ConfigManager checks both)
    let env_path = actual_base_dir.join(".env");
    fs::write(&env_path, env_content).expect("Failed to write .env to base dir");

    let manager = ConfigManager::new();
    let key = manager.get_api_key("test_provider");

    assert_eq!(key, Some("local_key_123".to_string()));

    // Clean up
    let _ = fs::remove_file(env_path);
    env::set_current_dir(original_cwd).expect("Failed to restore CWD");
}
