use llm_secure_cli::config::models::SecurityConfig;
use std::fs;
use std::sync::Mutex;
use tempfile::tempdir;

static TEST_LOCK: Mutex<()> = Mutex::new(());

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
        status: llm_secure_cli::security::audit::AuditStatus::Success,
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
