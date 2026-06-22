//! # Merkle Anchor Integrity Tests
//!
//! These tests target **SessionAnchorManager** — the component that links
//! Merkle Tree roots to PQC-signed anchor files for long-term tamper
//! detection (Tier 3: Time).
//!
//! ## Design principle
//!
//! We do NOT test MerkleTree internals (already covered by 7+ tests across
//! core_logic_tests, critical_integrity_tests, and pqc_tests).  Instead
//! we test the **create → verify → tamper → detect** lifecycle that
//! constitutes the actual security guarantee.
//!
//! ## Threat model
//!
//! Each scenario guards against a concrete post-compromise tampering
//! attempt that an attacker with filesystem access would attempt.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use llm_secure_cli::config::models::AppConfig;
use llm_secure_cli::security::audit::{AuditParams, log_audit_and_return};
use llm_secure_cli::security::merkle_anchor::SessionAnchorManager;
use std::fs;
use std::sync::Mutex;
use std::sync::Once;
use std::sync::OnceLock;
use tempfile::tempdir;

static ANCHOR_LOCK: Mutex<()> = Mutex::new(());
static TEST_ENV_INIT: Once = Once::new();
static _TEST_DIR: OnceLock<tempfile::TempDir> = OnceLock::new();

fn setup_test_env() {
    TEST_ENV_INIT.call_once(|| {
        let dir = tempdir().expect("Failed to create temp dir for test keys");
        let base_path = dir.path().to_path_buf();
        llm_secure_cli::consts::init_base_dir(Some(base_path));
        llm_secure_cli::security::identity::IdentityManager::ensure_keys_with_passphrase(None)
            .expect("Failed to generate PQC keys for test");
        let _ = _TEST_DIR.set(dir);
    });
}

fn write_entries(trace_id: &str, count: usize, config: &AppConfig) {
    let path = llm_secure_cli::consts::audit_log_path();
    for i in 0..count {
        let ctx = serde_json::json!({
            "trace_id": trace_id,
            "model": "test-model",
            "provider": "test-provider",
            "user_id": "test-user"
        });
        log_audit_and_return(
            AuditParams {
                event_type: "tool_call",
                tool_name: "test_tool",
                args: serde_json::json!({"seq": i}),
                output: Some("ok"),
                exit_code: Some(0),
                error: None,
                context: Some(&ctx),
                config,
            },
            Some(&path),
        );
    }
}

// =============================================================================
// 1. create_anchor → verify_session の完全ラウンドトリップ
// =============================================================================

#[test]
fn test_anchor_create_and_verify_happy_path() {
    let _lock = ANCHOR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    setup_test_env();

    let config = AppConfig::default();
    let trace_id = "test-session-001";

    write_entries(trace_id, 5, &config);

    // Create anchor
    let root = SessionAnchorManager::create_anchor(trace_id, None)
        .expect("create_anchor should succeed")
        .expect("create_anchor should return Some(root)");

    assert_eq!(root.len(), 64, "Merkle root must be 64 hex chars");
    assert!(
        root.chars().all(|c| c.is_ascii_hexdigit()),
        "Merkle root must be hex"
    );

    // Verify — should pass
    let verified =
        SessionAnchorManager::verify_session(trace_id).expect("verify_session should not error");
    assert!(
        verified,
        "Anchor verification must pass for unmodified data"
    );
}

#[test]
fn test_anchor_create_and_verify_with_pqc_signature() {
    let _lock = ANCHOR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    setup_test_env();

    let config = AppConfig::default();
    let trace_id = "test-session-pqc";

    write_entries(trace_id, 3, &config);

    let _root = SessionAnchorManager::create_anchor(trace_id, None)
        .expect("create_anchor should succeed")
        .expect("create_anchor should return Some(root)");

    // Verify the anchor file contains a PQC signature
    let anchor_dir = llm_secure_cli::consts::audit_log_path()
        .parent()
        .unwrap()
        .join("anchors");
    let anchor_path = anchor_dir.join(format!("{trace_id}.anchor.json"));
    assert!(anchor_path.exists(), "Anchor file must exist");

    let content = fs::read_to_string(&anchor_path).expect("Failed to read anchor file");
    let anchor: serde_json::Value =
        serde_json::from_str(&content).expect("Failed to parse anchor file");
    assert!(
        anchor
            .get("pqc_signature")
            .and_then(|s| s.as_str())
            .is_some(),
        "Anchor must have a PQC signature"
    );
    assert!(
        anchor
            .get("pqc_algorithm")
            .and_then(|s| s.as_str())
            .is_some(),
        "Anchor must specify the PQC algorithm"
    );

    // Verify should pass
    let verified =
        SessionAnchorManager::verify_session(trace_id).expect("verify_session should not error");
    assert!(verified, "PQC-signed anchor must verify successfully");
}

// =============================================================================
// 2. 改ざん検出
// =============================================================================

#[test]
fn test_anchor_verification_fails_when_entry_tampered() {
    let _lock = ANCHOR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    setup_test_env();

    let config = AppConfig::default();
    let trace_id = "test-session-tamper-entry";

    write_entries(trace_id, 3, &config);

    SessionAnchorManager::create_anchor(trace_id, None)
        .expect("create_anchor should succeed")
        .expect("create_anchor should return Some(root)");

    // Tamper with an entry in the log file: replace the last entry's output
    let log_path = llm_secure_cli::consts::audit_log_path();
    let content = fs::read_to_string(&log_path).expect("Failed to read log");
    let mut lines: Vec<String> = content.lines().map(String::from).collect();

    // Modify the last line's output
    if let Some(last) = lines.last_mut() {
        let mut entry: serde_json::Value =
            serde_json::from_str(last).expect("Failed to parse last entry");
        entry["output"] = serde_json::json!("TAMPERED_DATA");
        *last = serde_json::to_string(&entry).expect("Failed to serialize tampered entry");
    }
    fs::write(&log_path, lines.join("\n") + "\n").expect("Failed to write tampered log");

    // Verification must fail — hash chain broken
    let verified =
        SessionAnchorManager::verify_session(trace_id).expect("verify_session should not error");
    assert!(
        !verified,
        "Verification must fail when an audit entry has been tampered"
    );
}

#[test]
fn test_anchor_verification_fails_when_entry_count_changes() {
    let _lock = ANCHOR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    setup_test_env();

    let config = AppConfig::default();
    let trace_id = "test-session-tamper-count";

    write_entries(trace_id, 3, &config);

    SessionAnchorManager::create_anchor(trace_id, None)
        .expect("create_anchor should succeed")
        .expect("create_anchor should return Some(root)");

    // Add an extra entry after anchoring — changes entry count and chain
    let ctx = serde_json::json!({
        "trace_id": trace_id,
        "model": "test-model",
        "provider": "test-provider",
        "user_id": "test-user"
    });
    let log_path = llm_secure_cli::consts::audit_log_path();
    log_audit_and_return(
        AuditParams {
            event_type: "tool_call",
            tool_name: "extra_tool",
            args: serde_json::json!({"seq": 999}),
            output: Some("extra"),
            exit_code: Some(0),
            error: None,
            context: Some(&ctx),
            config: &config,
        },
        Some(&log_path),
    );

    // Verification must fail — entry count mismatch
    let verified =
        SessionAnchorManager::verify_session(trace_id).expect("verify_session should not error");
    assert!(
        !verified,
        "Verification must fail when entries have been added after anchoring"
    );
}

#[test]
fn test_anchor_verification_fails_when_anchor_merkle_root_tampered() {
    let _lock = ANCHOR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    setup_test_env();

    let config = AppConfig::default();
    let trace_id = "test-session-tamper-root";

    write_entries(trace_id, 3, &config);

    SessionAnchorManager::create_anchor(trace_id, None).expect("create_anchor should succeed");

    // Tamper with the anchor file: change the merkle_root
    let anchor_dir = llm_secure_cli::consts::audit_log_path()
        .parent()
        .unwrap()
        .join("anchors");
    let anchor_path = anchor_dir.join(format!("{trace_id}.anchor.json"));
    let mut anchor: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&anchor_path).expect("Failed to read anchor"))
            .expect("Failed to parse anchor");
    anchor["merkle_root"] =
        serde_json::json!("0000000000000000000000000000000000000000000000000000000000000000");
    fs::write(
        &anchor_path,
        serde_json::to_string_pretty(&anchor).expect("Failed to serialize tampered anchor"),
    )
    .expect("Failed to write tampered anchor");

    // Verification must detect the tampered merkle root
    let verified =
        SessionAnchorManager::verify_session(trace_id).expect("verify_session should not error");
    assert!(
        !verified,
        "Verification must fail when anchor merkle_root has been tampered"
    );
}

#[test]
fn test_anchor_verification_fails_when_pqc_signature_tampered() {
    let _lock = ANCHOR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    setup_test_env();

    let config = AppConfig::default();
    let trace_id = "test-session-tamper-sig";

    write_entries(trace_id, 3, &config);

    SessionAnchorManager::create_anchor(trace_id, None).expect("create_anchor should succeed");

    // Tamper with the PQC signature in the anchor file
    let anchor_dir = llm_secure_cli::consts::audit_log_path()
        .parent()
        .unwrap()
        .join("anchors");
    let anchor_path = anchor_dir.join(format!("{trace_id}.anchor.json"));
    let mut anchor: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&anchor_path).expect("Failed to read anchor"))
            .expect("Failed to parse anchor");
    // Flip a byte in the base64 signature
    if let Some(sig) = anchor["pqc_signature"].as_str() {
        let mut chars: Vec<char> = sig.chars().collect();
        if !chars.is_empty() {
            chars[0] = if chars[0] == 'A' { 'B' } else { 'A' };
        }
        anchor["pqc_signature"] = serde_json::json!(chars.iter().collect::<String>());
    }
    fs::write(
        &anchor_path,
        serde_json::to_string_pretty(&anchor).expect("Failed to serialize tampered anchor"),
    )
    .expect("Failed to write tampered anchor");

    // PQC signature verification should fail
    let verified =
        SessionAnchorManager::verify_session(trace_id).expect("verify_session should not error");
    assert!(
        !verified,
        "Verification must fail when PQC signature has been tampered"
    );
}

// =============================================================================
// 3. エッジケース
// =============================================================================

#[test]
fn test_anchor_nonexistent_session_returns_false() {
    let _lock = ANCHOR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    setup_test_env();

    let verified = SessionAnchorManager::verify_session("nonexistent-trace-id")
        .expect("verify_session should not error");
    assert!(
        !verified,
        "Non-existent session must return false, not error"
    );
}

#[test]
fn test_anchor_create_empty_session_returns_none() {
    let _lock = ANCHOR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    setup_test_env();

    let result = SessionAnchorManager::create_anchor("empty-session", None)
        .expect("create_anchor should not error for empty session");
    assert!(
        result.is_none(),
        "Empty session must return None, not Some(root)"
    );
}

#[test]
fn test_anchor_with_direct_entries_bypasses_log_scan() {
    let _lock = ANCHOR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    setup_test_env();

    // Build AuditEntries programmatically (no file I/O dependency)
    // to test Merkle tree construction and PQC signing in isolation.
    let entries = vec![serde_json::json!({
        "timestamp": "2026-01-01T00:00:00Z",
        "trace_id": "direct-session",
        "subject": "user",
        "audience": "-",
        "model": "test",
        "provider": "test",
        "event_type": "tool_call",
        "tool": "test_tool",
        "args": {"cmd": "ls"},
        "pqc_confidential": false,
        "status": "SUCCESS",
        "exit_code": 0,
        "prev_hash": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "hash": "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234",
        "hostname": "test",
        "os": "linux",
        "arch": "x86_64",
        "cli_version": "0.5.0"
    })];

    let root = SessionAnchorManager::create_anchor("direct-session", Some(entries))
        .expect("create_anchor with direct entries should succeed")
        .expect("create_anchor should return Some(root)");

    assert_eq!(root.len(), 64, "Root must be 64 hex chars");
    assert!(
        root.chars().all(|c| c.is_ascii_hexdigit()),
        "Root must be hex"
    );

    // Anchor file should be on disk
    let anchor_dir = llm_secure_cli::consts::audit_log_path()
        .parent()
        .unwrap()
        .join("anchors");
    let anchor_path = anchor_dir.join("direct-session.anchor.json");
    assert!(anchor_path.exists(), "Anchor file must exist");
}

#[test]
fn test_anchor_multiple_sessions_independent() {
    let _lock = ANCHOR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    setup_test_env();

    let config = AppConfig::default();
    let trace_a = "session-alpha";
    let trace_b = "session-beta";

    // Write different entries for each session
    {
        let ctx = serde_json::json!({
            "trace_id": trace_a, "model": "m", "provider": "p", "user_id": "u"
        });
        let log_path = llm_secure_cli::consts::audit_log_path();
        log_audit_and_return(
            AuditParams {
                event_type: "tool_call",
                tool_name: "alpha_tool",
                args: serde_json::json!({"data": "alpha"}),
                output: Some("ok"),
                exit_code: Some(0),
                error: None,
                context: Some(&ctx),
                config: &config,
            },
            Some(&log_path),
        );
    }
    {
        let ctx = serde_json::json!({
            "trace_id": trace_b, "model": "m", "provider": "p", "user_id": "u"
        });
        let log_path = llm_secure_cli::consts::audit_log_path();
        log_audit_and_return(
            AuditParams {
                event_type: "tool_call",
                tool_name: "beta_tool",
                args: serde_json::json!({"data": "beta"}),
                output: Some("ok"),
                exit_code: Some(0),
                error: None,
                context: Some(&ctx),
                config: &config,
            },
            Some(&log_path),
        );
    }

    // Create anchors for both
    let root_a = SessionAnchorManager::create_anchor(trace_a, None)
        .expect("create_anchor alpha")
        .expect("create_anchor alpha should return root");
    let root_b = SessionAnchorManager::create_anchor(trace_b, None)
        .expect("create_anchor beta")
        .expect("create_anchor beta should return root");

    // Different trace_ids must produce different merkle roots
    assert_ne!(
        root_a, root_b,
        "Different sessions must produce different merkle roots"
    );

    // Both must verify independently
    assert!(
        SessionAnchorManager::verify_session(trace_a).expect("verify alpha"),
        "Session alpha must verify"
    );
    assert!(
        SessionAnchorManager::verify_session(trace_b).expect("verify beta"),
        "Session beta must verify"
    );
}
