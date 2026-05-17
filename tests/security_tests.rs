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

// =============================================================================
// Unit tests for validate_tool_call (Phase 1 static analysis)
// =============================================================================

use llm_secure_cli::security::validate_tool_call;
use serde_json::{Map, Value, json};

fn make_args(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

#[test]
fn test_validate_null_byte_in_top_level_string_blocked() {
    let config = SecurityConfig::default();
    let args = make_args(&[("path", json!("file\0name.txt"))]);
    let result = validate_tool_call("read_file", &args, &config);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .contains("control characters or null bytes")
    );
}

#[test]
fn test_validate_control_char_bell_blocked() {
    let config = SecurityConfig::default();
    let args = make_args(&[("content", json!("hello\x07world"))]);
    let result = validate_tool_call("create_or_overwrite_file", &args, &config);
    assert!(result.is_err());
}

#[test]
fn test_validate_normal_strings_pass() {
    let config = SecurityConfig::default();
    let args = make_args(&[
        ("path", json!("file.txt")),
        ("content", json!("Hello world\nNew line\tTabbed")),
    ]);
    let result = validate_tool_call("edit_file", &args, &config);
    assert!(result.is_ok());
}

#[test]
fn test_validate_newline_tab_cr_pass() {
    let config = SecurityConfig::default();
    // \n, \r, \t are explicitly allowed
    let args = make_args(&[("code", json!("print('line1\nline2\rline3\tindented')"))]);
    let result = validate_tool_call("execute_python", &args, &config);
    assert!(result.is_ok());
}

#[test]
fn test_validate_non_string_values_pass() {
    let config = SecurityConfig::default();
    let args = make_args(&[
        ("count", json!(42)),
        ("flag", json!(true)),
        ("nothing", json!(null)),
    ]);
    let result = validate_tool_call("some_tool", &args, &config);
    assert!(result.is_ok());
}

#[test]
fn test_validate_empty_args_pass() {
    let config = SecurityConfig::default();
    let args = Map::new();
    let result = validate_tool_call("list_files", &args, &config);
    assert!(result.is_ok());
}

#[test]
fn test_validate_multiple_args_first_malicious_blocks() {
    let config = SecurityConfig::default();
    let args = make_args(&[
        ("safe", json!("normal")),
        ("bad", json!("null\0here")),
        ("also_safe", json!("fine")),
    ]);
    let result = validate_tool_call("test_tool", &args, &config);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("bad"));
}

#[test]
fn test_validate_unicode_text_passes() {
    let config = SecurityConfig::default();
    let args = make_args(&[("query", json!("日本語テスト 🦀"))]);
    let result = validate_tool_call("brave_search", &args, &config);
    assert!(result.is_ok());
}
