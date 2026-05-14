use llm_secure_cli::config::models::SecurityConfig;
use llm_secure_cli::security::path_validator::validate_path;
use std::env;
use std::fs;
use std::sync::Mutex;
use tempfile::tempdir;

static TEST_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn test_path_validation() {
    let _lock = TEST_LOCK.lock().expect("Failed to acquire test lock");
    let dir = tempdir().expect("Failed to create temp dir");
    let original_dir = env::current_dir().expect("Failed to get current dir");
    env::set_current_dir(dir.path()).expect("Failed to set current dir");

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

    // 2. Traversal path (normalized and then checked against allowed_paths)
    let res = validate_path("../outside.txt", &config);
    // Should now return Err because it resolves to a path outside current directory
    assert!(res.is_err());

    // 3. Absolute path
    let res = validate_path("/etc/passwd", &config);
    // Should now return Err because it's not within "." (current directory)
    assert!(res.is_err());

    // 4. Normalization
    let res = validate_path("  'sub/dir/'  ", &config);
    assert!(res.is_ok());
    let path_str = res
        .expect("path should be valid")
        .to_str()
        .expect("path should be valid UTF-8")
        .replace("\\", "/");
    assert!(path_str.contains("sub/dir"));

    env::set_current_dir(original_dir).expect("Failed to restore current dir");
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
        provider: "openai".to_string(),
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
        hostname: "test-host".to_string(),
        os: "linux".to_string(),
        arch: "x86_64".to_string(),
        cli_version: "0.1.0".to_string(),
    };

    let json = serde_json::to_string(&entry).expect("Serialization should succeed");
    assert!(json.contains(r#""trace_id":"test-trace""#));
    assert!(json.contains(r#""hash":"hash1""#));
}

#[test]
fn test_audit_hash_chaining() {
    let _lock = TEST_LOCK.lock().expect("Failed to acquire test lock");
    use llm_secure_cli::config::models::AppConfig;
    use llm_secure_cli::security::audit::log_audit_and_return;

    let dir = tempdir().expect("Failed to create temp dir");
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
        let entry1: serde_json::Value =
            serde_json::from_str(lines[lines.len() - 2]).expect("Failed to parse entry1");
        let entry2: serde_json::Value =
            serde_json::from_str(lines[lines.len() - 1]).expect("Failed to parse entry2");

        let hash1 = entry1
            .get("hash")
            .expect("entry1 should have hash")
            .as_str()
            .expect("hash should be a string");
        let prev_hash2 = entry2
            .get("prev_hash")
            .expect("entry2 should have prev_hash")
            .as_str()
            .expect("prev_hash should be a string");

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

#[test]
fn test_mcp_security_validation() {
    use llm_secure_cli::security::validate_tool_call;
    use serde_json::json;

    let config = SecurityConfig {
        allowed_paths: vec!["/tmp/allowed".to_string()],
        ..SecurityConfig::default()
    };

    // 1. MCP-like tool name with path traversal
    let args = json!({
        "server_root": "/etc/passwd"
    });
    let res = validate_tool_call(
        "filesystem__read_file",
        args.as_object().expect("args should be an object"),
        &config,
    );
    // Now returns Ok, delegated to Phase 3 (Dual LLM)
    assert!(res.is_ok());

    // 2. MCP-like command execution with malicious characters
    let args = json!({
        "cmd": "ls",
        "args": ["normal_arg", "with\0null"]
    });
    let res = validate_tool_call(
        "shell__run_shell",
        args.as_object().expect("args should be an object"),
        &config,
    );
    // Now returns Ok, delegated to Phase 3 (Dual LLM)
    assert!(res.is_ok());
}
