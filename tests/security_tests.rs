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

    // 3. Blocked by being outside allowed roots (absolute)
    let res = validate_path("/etc/passwd");
    assert!(res.is_err());
    assert!(res.unwrap_err().0.contains("outside allowed directories"));

    // 4. Normalization
    let res = validate_path("  'sub/dir/'  ");
    assert!(res.is_ok());
    let path_str = res.unwrap().to_str().unwrap().replace("\\", "/");
    assert!(path_str.contains("sub/dir"));

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

#[test]
fn test_audit_hash_chaining() {
    use llm_secure_cli::consts::AUDIT_LOG_PATH;
    use llm_secure_cli::security::audit::log_audit;

    let path = &*AUDIT_LOG_PATH;

    // Backup existing log if any
    let backup = if path.exists() {
        let content = fs::read_to_string(path).unwrap();
        fs::remove_file(path).unwrap();
        Some(content)
    } else {
        None
    };

    // Ensure directory exists
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    // Call log_audit twice.
    // The second call should pick up the hash of the first call as its prev_hash.
    log_audit(
        "tool_call",
        "test_tool",
        serde_json::json!({}),
        None,
        Some(0),
        None,
        None,
    );
    log_audit(
        "tool_call",
        "test_tool",
        serde_json::json!({}),
        None,
        Some(0),
        None,
        None,
    );

    // Read the log file
    let content = fs::read_to_string(path).expect("Failed to read audit log");
    let lines: Vec<&str> = content.lines().collect();

    if lines.len() >= 2 {
        let entry1: serde_json::Value = serde_json::from_str(lines[lines.len() - 2]).unwrap();
        let entry2: serde_json::Value = serde_json::from_str(lines[lines.len() - 1]).unwrap();

        let hash1 = entry1.get("hash").unwrap().as_str().unwrap();
        let prev_hash2 = entry2.get("prev_hash").unwrap().as_str().unwrap();

        assert_eq!(
            hash1, prev_hash2,
            "Hash chain broken! Prev hash of second entry should match hash of first entry."
        );
        assert_ne!(
            prev_hash2,
            "0".repeat(64),
            "Prev hash should not be all 0s for second entry"
        );
    } else {
        panic!(
            "Should have at least 2 audit entries, found {}",
            lines.len()
        );
    }

    // Cleanup and restore
    fs::remove_file(path).unwrap();
    if let Some(c) = backup {
        fs::write(path, c).unwrap();
    }
}
