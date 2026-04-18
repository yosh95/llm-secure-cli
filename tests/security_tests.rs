use llm_secure_cli::config::CONFIG_MANAGER;
use llm_secure_cli::security::path_validator::validate_path;
use std::env;
use std::fs;
use tempfile::tempdir;

#[test]
fn test_path_validation() {
    let dir = tempdir().unwrap();
    let original_dir = env::current_dir().unwrap();
    env::set_current_dir(dir.path()).unwrap();

    // Create a dummy config.toml in the temp dir
    let config_content = r#"
[security]
allowed_paths = ["."]
blocked_paths = ["/etc"]
blocked_filenames = ["*.key", ".env"]
"#;
    fs::write(dir.path().join("config.toml"), config_content).unwrap();

    // Reload config to pick up the new config.toml
    CONFIG_MANAGER.reload();

    // 1. Allowed path (current directory)
    let res = validate_path("test.txt");
    assert!(
        res.is_ok(),
        "Should allow test.txt in CWD, got {:?}",
        res.err()
    );

    // 2. Blocked by traversal
    let res = validate_path("../outside.txt");
    assert!(res.is_err());
    assert!(res.unwrap_err().0.contains("traversal"));

    // 3. Blocked by absolute path
    let res = validate_path("/etc/passwd");
    assert!(res.is_err());
    assert!(res.unwrap_err().0.contains("blocked path"));

    // 4. Blocked by filename pattern
    let res = validate_path("secret.key");
    assert!(res.is_err());
    assert!(res.unwrap_err().0.contains("forbidden"));

    let res = validate_path(".env");
    assert!(res.is_err());

    // 5. Normalization
    let res = validate_path("  'sub/dir/'  ");
    assert!(res.is_ok());
    assert!(res.unwrap().to_str().unwrap().ends_with("sub/dir"));

    env::set_current_dir(original_dir).unwrap();
}

#[test]
fn test_audit_entry_serialization() {
    use llm_secure_cli::security::audit::AuditEntry;

    let entry = AuditEntry {
        timestamp: "2023-01-01T00:00:00Z".to_string(),
        trace_id: "test-trace".to_string(),
        subject: "user".to_string(),
        audience: "system".to_string(),
        model: "gpt-4".to_string(),
        event_type: "test".to_string(),
        tool: "test_tool".to_string(),
        args: serde_json::json!({"arg1": "val1"}),
        pqc_confidential: false,
        output: Some("test output".to_string()),
        status: "SUCCESS".to_string(),
        exit_code: Some(0),
        prev_hash: "0".repeat(64),
        hash: "hash1".to_string(),
        pqc_signature: None,
        pqc_algorithm: None,
    };

    let json = serde_json::to_string(&entry).unwrap();
    assert!(json.contains("\"trace_id\":\"test-trace\""));
    assert!(json.contains("\"hash\":\"hash1\""));
}
