use llm_secure_cli::config::models::SecurityConfig;
use llm_secure_cli::security::path_validator::validate_path;
use std::env;
use std::fs;
use std::sync::Mutex;
use tempfile::tempdir;

static TEST_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn test_path_validation() {
    let dir = tempdir().unwrap();
    let original_dir = env::current_dir().unwrap();
    env::set_current_dir(dir.path()).unwrap();

    let config = SecurityConfig {
        allowed_paths: vec![".".to_string()],
        ..SecurityConfig::default()
    };

    // 1. Allowed path (current directory)
    let res = validate_path("test.txt", &config);
    assert!(
        res.is_ok(),
        "Should allow test.txt in CWD, got {:?}",
        res.err()
    );

    // 2. Blocked by traversal
    let res = validate_path("../outside.txt", &config);
    assert!(res.is_err());
    assert!(res.unwrap_err().0.contains("traversal"));

    // 3. Blocked by being outside allowed roots (absolute)
    let res = validate_path("/etc/passwd", &config);
    assert!(res.is_err());
    assert!(res.unwrap_err().0.contains("outside allowed directories"));

    // 4. Normalization
    let res = validate_path("  'sub/dir/'  ", &config);
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
    let _lock = TEST_LOCK.lock().unwrap();
    use llm_secure_cli::config::models::AppConfig;
    use llm_secure_cli::security::audit::log_audit_and_return;

    let dir = tempdir().unwrap();
    let path = dir.path().join("audit_test.jsonl");
    let config = AppConfig::default();

    // Call log_audit_and_return twice.
    // The second call should pick up the hash of the first call as its prev_hash.
    use llm_secure_cli::security::audit::AuditParams;
    log_audit_and_return(
        AuditParams {
            event_type: "tool_call",
            tool_name: "test_tool",
            args: serde_json::json!({}),
            output: None,
            exit_code: Some(0),
            error: None,
            context: None,
            config: &config,
        },
        Some(&path),
    );
    log_audit_and_return(
        AuditParams {
            event_type: "tool_call",
            tool_name: "test_tool",
            args: serde_json::json!({}),
            output: None,
            exit_code: Some(0),
            error: None,
            context: None,
            config: &config,
        },
        Some(&path),
    );

    // Read the log file
    let content = fs::read_to_string(&path).expect("Failed to read audit log");
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
}
