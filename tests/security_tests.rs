#![allow(clippy::unwrap_used, clippy::expect_used)]
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

// =============================================================================
// Additional unit tests for AuditStatus (Display, TryFrom, From)
// =============================================================================

#[test]
fn test_audit_status_display_success() {
    use llm_secure_cli::security::audit::AuditStatus;
    assert_eq!(AuditStatus::Success.to_string(), "SUCCESS");
}

#[test]
fn test_audit_status_display_failed() {
    use llm_secure_cli::security::audit::AuditStatus;
    assert_eq!(
        AuditStatus::Failed("permission denied".to_string()).to_string(),
        "FAILED: permission denied"
    );
}

#[test]
fn test_audit_status_display_pqc_encryption_failed() {
    use llm_secure_cli::security::audit::AuditStatus;
    assert_eq!(
        AuditStatus::PqcEncryptionFailed("key not found".to_string()).to_string(),
        "FAILED: key not found; PQC_ENCRYPTION_FAILED"
    );
}

#[test]
fn test_audit_status_display_integrity_failure() {
    use llm_secure_cli::security::audit::AuditStatus;
    assert_eq!(
        AuditStatus::IntegrityFailure("hash mismatch".to_string()).to_string(),
        "INTEGRITY_FAILURE: hash mismatch"
    );
}

#[test]
fn test_audit_status_display_success_without_signature() {
    use llm_secure_cli::security::audit::AuditStatus;
    assert_eq!(
        AuditStatus::SuccessWithoutSignature.to_string(),
        "SUCCESS_WITHOUT_SIGNATURE: PQC private key unavailable"
    );
}

#[test]
fn test_audit_status_display_log_rotation_marker() {
    use llm_secure_cli::security::audit::AuditStatus;
    assert_eq!(
        AuditStatus::LogRotationMarker.to_string(),
        "CONTINUITY_MAINTAINED"
    );
}

#[test]
fn test_audit_status_try_from_success() {
    use llm_secure_cli::security::audit::AuditStatus;
    let result: Result<AuditStatus, String> = "SUCCESS".to_string().try_into();
    assert!(matches!(result, Ok(AuditStatus::Success)));
}

#[test]
fn test_audit_status_try_from_failed() {
    use llm_secure_cli::security::audit::AuditStatus;
    let result: Result<AuditStatus, String> = "FAILED: something broke".to_string().try_into();
    assert!(matches!(result, Ok(AuditStatus::Failed(_))));
}

#[test]
fn test_audit_status_try_from_integrity_failure() {
    use llm_secure_cli::security::audit::AuditStatus;
    let result: Result<AuditStatus, String> =
        "INTEGRITY_FAILURE: hash mismatch".to_string().try_into();
    assert!(matches!(result, Ok(AuditStatus::IntegrityFailure(_))));
}

#[test]
fn test_audit_status_try_from_pqc_encryption_failed() {
    use llm_secure_cli::security::audit::AuditStatus;
    let s = "FAILED: encryption error; PQC_ENCRYPTION_FAILED".to_string();
    let result: Result<AuditStatus, String> = s.try_into();
    assert!(matches!(result, Ok(AuditStatus::PqcEncryptionFailed(_))));
}

#[test]
fn test_audit_status_try_from_log_rotation() {
    use llm_secure_cli::security::audit::AuditStatus;
    let result: Result<AuditStatus, String> = "CONTINUITY_MAINTAINED".to_string().try_into();
    assert!(matches!(result, Ok(AuditStatus::LogRotationMarker)));
}

#[test]
fn test_audit_status_try_from_success_without_sig() {
    use llm_secure_cli::security::audit::AuditStatus;
    let result: Result<AuditStatus, String> =
        "SUCCESS_WITHOUT_SIGNATURE: PQC private key unavailable"
            .to_string()
            .try_into();
    assert!(matches!(result, Ok(AuditStatus::SuccessWithoutSignature)));
}

#[test]
fn test_audit_status_try_from_unknown_fallback() {
    use llm_secure_cli::security::audit::AuditStatus;
    // Unknown statuses should fall back to Failed (forward-compatible)
    let result: Result<AuditStatus, String> = "UNKNOWN_STATUS".to_string().try_into();
    assert!(matches!(result, Ok(AuditStatus::Failed(_))));
}

#[test]
fn test_audit_status_roundtrip_serde() {
    use llm_secure_cli::security::audit::AuditStatus;
    let variants = vec![
        AuditStatus::Success,
        AuditStatus::Failed("error".to_string()),
        AuditStatus::PqcEncryptionFailed("pqc_err".to_string()),
        AuditStatus::IntegrityFailure("int_err".to_string()),
        AuditStatus::SuccessWithoutSignature,
        AuditStatus::LogRotationMarker,
    ];
    for v in variants {
        let json = serde_json::to_value(&v).expect("serialise");
        let back: AuditStatus = serde_json::from_value(json).expect("deserialise");
        assert_eq!(v.to_string(), back.to_string());
    }
}

// =============================================================================
// Tests for AuditParamsBuilder (integration with log_audit_and_return)
// =============================================================================

#[test]
fn test_audit_params_builder_produces_valid_log_entry() {
    let _lock = TEST_LOCK.lock().expect("Failed to acquire test lock");
    use llm_secure_cli::config::models::AppConfig;
    use llm_secure_cli::security::audit::{AuditParamsBuilder, AuditStatus};

    let dir = tempdir().expect("Failed to create temp dir");
    let path = dir.path().join("builder_test.jsonl");
    let config = AppConfig::default();
    let ctx = serde_json::json!({"env": "test"});

    let result = AuditParamsBuilder::new("custom_event", "custom_tool", &config)
        .args(serde_json::json!({"key": "value"}))
        .output("builder output")
        .exit_code(0)
        .context(&ctx)
        .log_and_return(Some(&path));

    assert!(result.is_some(), "Builder should produce a log entry");
    let entry = result.unwrap();
    assert_eq!(entry.event_type, "custom_event");
    assert!(matches!(entry.status, AuditStatus::Success));
    assert_eq!(entry.tool, "custom_tool");
}

#[test]
fn test_audit_params_builder_with_error() {
    let _lock = TEST_LOCK.lock().expect("Failed to acquire test lock");
    use llm_secure_cli::config::models::AppConfig;
    use llm_secure_cli::security::audit::{AuditParamsBuilder, AuditStatus};

    let dir = tempdir().expect("Failed to create temp dir");
    let path = dir.path().join("builder_error.jsonl");
    let config = AppConfig::default();

    let result = AuditParamsBuilder::new("build_event", "failing_tool", &config)
        .error("something went wrong")
        .exit_code(1)
        .log_and_return(Some(&path));

    assert!(result.is_some());
    let entry = result.unwrap();
    assert!(matches!(entry.status, AuditStatus::Failed(_)));
    assert_eq!(entry.exit_code, Some(1));
}

// =============================================================================
// Tests for log_audit (non-returning) and log_audit_and_return (error handling)
// =============================================================================

#[test]
fn test_log_audit_non_returning() {
    let _lock = TEST_LOCK.lock().expect("Failed to acquire test lock");
    use llm_secure_cli::config::models::AppConfig;
    use llm_secure_cli::security::audit::{AuditParams, log_audit, log_audit_and_return};

    let dir = tempdir().expect("Failed to create temp dir");
    let path = dir.path().join("audit_nonreturn.jsonl");
    let config = AppConfig::default();

    // log_audit (non-returning) — verifies it doesn't panic
    log_audit(AuditParams {
        event_type: "test_event",
        tool_name: "test_tool",
        args: serde_json::json!({"key": "val"}),
        output: Some("output"),
        exit_code: Some(0),
        error: None,
        context: None,
        config: &config,
    });

    // Use log_audit_and_return to verify the file gets created
    let entry = log_audit_and_return(
        AuditParams {
            event_type: "test_event",
            tool_name: "test_tool",
            args: serde_json::json!({"key": "val"}),
            output: Some("output"),
            exit_code: Some(0),
            error: None,
            context: None,
            config: &config,
        },
        Some(&path),
    );
    assert!(entry.is_some(), "Should return an audit entry");
    assert!(path.exists(), "Audit log file should exist");
    let content = fs::read_to_string(&path).expect("Failed to read audit log");
    assert!(!content.is_empty(), "Audit log should not be empty");
    assert!(
        content.contains("test_event"),
        "Log should contain event_type"
    );
    assert!(
        content.contains("test_tool"),
        "Log should contain tool_name"
    );
}

#[test]
fn test_log_audit_and_return_with_error_status() {
    let _lock = TEST_LOCK.lock().expect("Failed to acquire test lock");
    use llm_secure_cli::config::models::AppConfig;
    use llm_secure_cli::security::audit::{AuditParams, AuditStatus, log_audit_and_return};

    let dir = tempdir().expect("Failed to create temp dir");
    let path = dir.path().join("audit_error.jsonl");
    let config = AppConfig::default();

    let result = log_audit_and_return(
        AuditParams {
            event_type: "tool_call",
            tool_name: "failing_tool",
            args: serde_json::json!({"arg": "value"}),
            output: Some("error: permission denied"),
            exit_code: Some(1),
            error: Some("permission denied"),
            context: None,
            config: &config,
        },
        Some(&path),
    );

    assert!(result.is_some(), "Should return an AuditEntry");
    let entry = result.unwrap();
    assert!(matches!(entry.status, AuditStatus::Failed(_)));
    assert_eq!(entry.exit_code, Some(1));
    assert_eq!(entry.tool, "failing_tool");
}

#[test]
fn test_log_audit_and_return_with_success_status() {
    let _lock = TEST_LOCK.lock().expect("Failed to acquire test lock");
    use llm_secure_cli::config::models::AppConfig;
    use llm_secure_cli::security::audit::{AuditParams, AuditStatus, log_audit_and_return};

    let dir = tempdir().expect("Failed to create temp dir");
    let path = dir.path().join("audit_success.jsonl");
    let config = AppConfig::default();

    let result = log_audit_and_return(
        AuditParams {
            event_type: "tool_call",
            tool_name: "success_tool",
            args: serde_json::json!({}),
            output: Some("completed successfully"),
            exit_code: Some(0),
            error: None,
            context: None,
            config: &config,
        },
        Some(&path),
    );

    assert!(result.is_some(), "Should return an AuditEntry");
    let entry = result.unwrap();
    assert!(matches!(entry.status, AuditStatus::Success));
    assert_eq!(entry.exit_code, Some(0));
    assert_eq!(entry.tool, "success_tool");
}
