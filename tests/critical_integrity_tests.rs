//! # Critical Integrity Tests
//!
//! These tests target **high-impact, under-tested areas** where bugs would cause
//! real-world harm: LLM-facing output formatting, audit chain integrity,
//! security boundary enforcement, and cryptographic storage.
//!
//! This is NOT about coverage vanity — each test below guards against a
//! specific failure mode that was identified through code review.

use llm_secure_cli::tools::executor_utils::{humanize_tool_result, truncate_output};
use serde_json::json;

// =============================================================================
// humanize_tool_result — LLM-facing output formatter
// =============================================================================
//
// This function converts every tool's JSON result into a human-readable string
// that the LLM sees as its "observation."  If this function produces garbled,
// truncated, or wrong output, the LLM will hallucinate incorrect decisions.
//
// Each branch (file_ops, grep, search, python, brave_search, etc.) is tested.

#[test]
fn test_humanize_grep_results_formats_correctly() {
    let v = json!({
        "matches": [
            {"file": "src/main.rs", "line": 42, "text": "fn main() {"},
            {"file": "src/lib.rs", "line": 7, "text": "pub mod config;"}
        ]
    });
    let output = humanize_tool_result("grep_files", &v);
    assert!(output.contains("Found 2 matches"));
    assert!(output.contains("src/main.rs:42: fn main() {"));
    assert!(output.contains("src/lib.rs:7: pub mod config;"));
    assert!(!output.contains("[TRUNCATED]"));
}

#[test]
fn test_humanize_grep_empty_results_shows_message() {
    let v = json!({"matches": [], "message": "No matches found."});
    let output = humanize_tool_result("grep_files", &v);
    assert_eq!(output, "No matches found.");
}

#[test]
fn test_humanize_grep_truncated_flag_shown() {
    let v = json!({
        "matches": [{"file": "a.rs", "line": 1, "text": "// hi"}],
        "truncated": true
    });
    let output = humanize_tool_result("grep_files", &v);
    assert!(output.contains("... (results truncated)"));
}

#[test]
fn test_humanize_list_files_shows_items() {
    let v = json!({
        "files": [
            {"path": "src/main.rs", "type": "file"},
            {"path": "src/lib.rs", "type": "file"},
            {"path": "images", "type": "dir"}
        ]
    });
    let output = humanize_tool_result("list_files", &v);
    assert!(output.contains("Found 3 items"));
    assert!(output.contains("[file] src/main.rs"));
    assert!(output.contains("[dir] images"));
}

#[test]
fn test_humanize_list_files_empty() {
    let v = json!({"files": [], "message": "No items found."});
    let output = humanize_tool_result("list_files", &v);
    assert_eq!(output, "No items found.");
}

#[test]
fn test_humanize_search_files_shows_matches() {
    let v = json!({
        "results": [
            {"path": "Cargo.toml", "type": "file"},
            {"path": "Cargo.lock", "type": "file"}
        ]
    });
    let output = humanize_tool_result("search_files", &v);
    assert!(output.contains("Found 2 items"));
    assert!(output.contains("[file] Cargo.toml"));
}

#[test]
fn test_humanize_python_execution_shows_stdout_stderr() {
    let v = json!({
        "stdout": "Hello World
",
        "stderr": "",
        "exit_code": 0
    });
    let output = humanize_tool_result("execute_python", &v);
    assert!(output.contains("Exit Code: 0"));
    assert!(output.contains("STDOUT:"));
    assert!(output.contains("Hello World"));
    assert!(!output.contains("STDERR:"));
}

#[test]
fn test_humanize_python_execution_with_errors() {
    let v = json!({
        "stdout": "",
        "stderr": "Traceback (most recent call last):\n  File \"<stdin>\", line 1, in <module>\nNameError: name 'x' is not defined\n",
        "exit_code": 1
    });
    let output = humanize_tool_result("execute_python", &v);
    assert!(output.contains("Exit Code: 1"));
    assert!(!output.contains("STDOUT:"));
    assert!(output.contains("STDERR:"));
    assert!(output.contains("NameError"));
}

#[test]
fn test_humanize_edit_file_shows_diff() {
    let v = json!({
        "success": true,
        "path": "/tmp/test.txt",
        "message": "File updated successfully.",
        "diff": "--- a/test.txt
+++ b/test.txt
@@ -1 +1 @@
-old
+new"
    });
    let output = humanize_tool_result("edit_file", &v);
    assert!(output.contains("File updated successfully."));
    assert!(output.contains("File: /tmp/test.txt"));
    assert!(output.contains("--- Diff ---"));
    assert!(output.contains("-old"));
    assert!(output.contains("+new"));
    assert!(output.contains("------------"));
}

#[test]
fn test_humanize_create_file_shows_diff() {
    let v = json!({
        "success": true,
        "path": "/tmp/new.txt",
        "message": "File created successfully.",
        "diff": "@@ -0,0 +1 @@
+new content"
    });
    let output = humanize_tool_result("create_or_overwrite_file", &v);
    assert!(output.contains("File created successfully."));
    assert!(output.contains("File: /tmp/new.txt"));
    assert!(output.contains("--- Diff ---"));
}

#[test]
fn test_humanize_brave_search_formats_correctly() {
    let v = json!({
        "query": "rust programming",
        "results": [
            {
                "title": "Rust Lang",
                "url": "https://www.rust-lang.org",
                "snippets": ["A language empowering everyone to build reliable software."]
            },
            {
                "title": "Learn Rust",
                "url": "https://learn.rust-lang.org",
                "snippets": ["An interactive book.", "With exercises."]
            }
        ]
    });
    let output = humanize_tool_result("brave_search", &v);
    let expected_prefix = "Search results for \"rust programming\" (2 items)";
    assert!(output.starts_with(expected_prefix),
        "Expected prefix: {:?}, got: {:?}", expected_prefix, &output[..expected_prefix.len().min(output.len())]);
    assert!(output.contains("1. Rust Lang"));
    assert!(output.contains("URL: https://www.rust-lang.org"));
    assert!(output.contains("2. Learn Rust"));
    assert!(output.contains("An interactive book."));
    assert!(output.contains("With exercises."));
}

#[test]
fn test_humanize_brave_search_no_results() {
    let v = json!({"query": "nothing", "results": []});
    let output = humanize_tool_result("brave_search", &v);
    assert_eq!(output, "No search results found.");
}

#[test]
fn test_humanize_fallback_shows_pretty_json_for_objects() {
    let v = json!({"custom": "data", "nested": {"key": "val"}});
    let output = humanize_tool_result("unknown_tool", &v);
    // Should produce pretty-printed JSON with indented keys
    assert!(output.contains("custom"), "output should contain key 'custom': {}", output);
    assert!(output.contains("nested"), "output should contain key 'nested': {}", output);
    assert!(output.contains("data"), "output should contain value 'data': {}", output);
    assert!(output.contains("val"), "output should contain value 'val': {}", output);
    // Should have newlines (pretty-print) or be structured
    assert!(output.contains('\n') || output.contains('{'),
        "output should be structured JSON: {}", output);
}

#[test]
fn test_humanize_fallback_shows_string_directly() {
    let v = json!("Just a plain string result");
    let output = humanize_tool_result("some_tool", &v);
    assert_eq!(output, "Just a plain string result");
}

#[test]
fn test_humanize_fallback_shows_number() {
    let v = json!(42);
    let output = humanize_tool_result("count_tool", &v);
    assert_eq!(output, "42");
}

// =============================================================================
// truncate_output — Output truncation edge cases
// =============================================================================
//
// This function is the last line of defense against runaway output.
// Bugs here can cause panics (unicode boundary), data loss, or
// misleading truncation messages.

#[test]
fn test_truncate_output_under_limit_unchanged() {
    let input = "Hello, world!";
    let result = truncate_output(&input, 100, 1000);
    assert_eq!(result, input);
}

#[test]
fn test_truncate_output_exactly_at_line_limit() {
    let input = "line1
line2
line3";
    let result = truncate_output(&input, 3, 1000);
    // Exactly 3 lines — should not truncate
    assert_eq!(result, input);
}

#[test]
fn test_truncate_output_one_line_over_limit() {
    let input = "line1
line2
line3
line4";
    let result = truncate_output(&input, 3, 1000);
    assert!(result.contains("line1"));
    assert!(result.contains("line2"));
    assert!(result.contains("line3"));
    assert!(!result.contains("line4")); // truncated
    assert!(result.contains("Shown 3 of 4 lines"));
}

#[test]
fn test_truncate_output_exactly_at_char_limit() {
    let input = "a".repeat(100);
    let result = truncate_output(&input, 1000, 100);
    assert_eq!(result.len(), 100); // exactly at limit
}

#[test]
fn test_truncate_output_one_char_over_limit() {
    let input = "a".repeat(101);
    let result = truncate_output(&input, 1000, 100);
    // The content before the truncation message must be exactly 100 chars
    let trunc_msg_start = result.find("... (Output truncated").unwrap_or(result.len());
    let content_part = &result[..trunc_msg_start];
    assert_eq!(content_part.chars().filter(|&c| c == 'a').count(), 100,
        "Content before truncation message should be 100 chars");
    // Verify the message mentions truncation occurred
    assert!(result.contains("(Output truncated"),
        "Result must indicate truncation: {:?}", result);
    // Verify we see the original and shown counts somewhere
    assert!(result.contains("100 of") || result.contains("101"),
        "Should show char counts: {:?}", result);
}

#[test]
fn test_truncate_output_unicode_boundary_no_panic() {
    // é (U+00E1) is 2 bytes in UTF-8.
    // If truncation cuts in the middle of the byte sequence, it must
    // adjust to the nearest char boundary.
    let input = "a".repeat(98) + "é"; // 99 chars, 100 bytes
    let result = truncate_output(&input, 1000, 99);
    // Must not panic, must produce valid UTF-8
    assert!(result.is_char_boundary(result.len()));
}

#[test]
fn test_truncate_output_emoji_boundary_no_panic() {
    // 🦀 (Rust crab) is 4 bytes. Truncation must not split it.
    let input = "a".repeat(97) + "🦀"; // 98 chars
    let result = truncate_output(&input, 1000, 98);
    assert!(result.is_char_boundary(result.len()));
    // The emoji should either be fully present or fully absent, never half
    if result.contains('🦀') {
        assert!(result.ends_with('🦀') || result.contains("..."));
    }
}

#[test]
fn test_truncate_output_empty_string() {
    let result = truncate_output("", 100, 1000);
    assert_eq!(result, "");
}

#[test]
fn test_truncate_output_zero_lines_truncates_all() {
    let input = "some content";
    let result = truncate_output(&input, 0, 1000);
    assert!(result.contains("Shown 0 of"));
}

#[test]
fn test_truncate_output_both_limits_hit() {
    let input = "short
".repeat(200); // 200 lines, ~1200 chars
    let result = truncate_output(&input, 50, 500);
    // Both limits exceeded — the most restrictive applies
    assert!(result.contains("Shown"));
    assert!(result.contains("lines"));
    assert!(result.contains("chars"));
}

#[test]
fn test_truncate_output_truncation_message_format() {
    let input = "line1
line2
line3
line4
line5";
    let result = truncate_output(&input, 3, 1000);
    let msg_line = result.lines().last().unwrap();
    assert!(msg_line.starts_with("... (Output truncated."));
    assert!(msg_line.contains("Shown 3 of 5 lines"));
    assert!(msg_line.contains(" of "));
}

// =============================================================================
// AuditStatus — Full serialisation round-trip for ALL variants
// =============================================================================
//
// AuditStatus is the backbone of the audit chain integrity.
// Every variant must survive serialise → deserialise unchanged.
// A bug here would break forensic analysis and Merkle verification.

use llm_secure_cli::security::audit::AuditStatus;

#[test]
fn test_audit_status_success_round_trip() {
    let status = AuditStatus::Success;
    let s: String = status.clone().into();
    let back: AuditStatus = s.try_into().expect("Should deserialise");
    assert_eq!(status, back);
}

#[test]
fn test_audit_status_failed_round_trip() {
    let status = AuditStatus::Failed("File not found: /etc/shadow".to_string());
    let s: String = status.clone().into();
    let back: AuditStatus = s.try_into().expect("Should deserialise");
    assert_eq!(status, back);
}

#[test]
fn test_audit_status_failed_with_colon_in_message_round_trip() {
    // "FAILED: error: something" — the colon after FAILED could confuse parsing
    let status = AuditStatus::Failed("error: permission denied".to_string());
    let s: String = status.clone().into();
    let back: AuditStatus = s.try_into().expect("Should deserialise");
    assert_eq!(status, back);
}

#[test]
fn test_audit_status_pqc_encryption_failed_round_trip() {
    let status = AuditStatus::PqcEncryptionFailed("KEM encapsulate failed".to_string());
    let s: String = status.clone().into();
    assert!(s.contains("PQC_ENCRYPTION_FAILED"));
    let back: AuditStatus = s.try_into().expect("Should deserialise PQC_ENCRYPTION_FAILED");
    assert_eq!(status, back);
}

#[test]
fn test_audit_status_integrity_failure_round_trip() {
    let status = AuditStatus::IntegrityFailure("PQC Sign failed: network error".to_string());
    let s: String = status.clone().into();
    assert!(s.starts_with("INTEGRITY_FAILURE:"));
    let back: AuditStatus = s.try_into().expect("Should deserialise INTEGRITY_FAILURE");
    assert_eq!(status, back);
}

#[test]
fn test_audit_status_success_without_signature_round_trip() {
    let status = AuditStatus::SuccessWithoutSignature;
    let s: String = status.clone().into();
    assert!(s.starts_with("SUCCESS_WITHOUT_SIGNATURE"));
    let back: AuditStatus = s.try_into().expect("Should deserialise");
    assert_eq!(status, back);
}

#[test]
fn test_audit_status_log_rotation_marker_round_trip() {
    let status = AuditStatus::LogRotationMarker;
    let s: String = status.clone().into();
    assert_eq!(s, "CONTINUITY_MAINTAINED");
    let back: AuditStatus = s.try_into().expect("Should deserialise LogRotationMarker");
    assert_eq!(status, back);
}

#[test]
fn test_audit_status_unknown_status_wrapped_in_failed() {
    // Forward compatibility: unknown status strings are wrapped in Failed
    let s = "SOME_FUTURE_STATUS".to_string();
    let back: AuditStatus = s.try_into().expect("Should handle unknown status");
    assert_eq!(back, AuditStatus::Failed("SOME_FUTURE_STATUS".to_string()));
}

#[test]
fn test_audit_status_display_success() {
    assert_eq!(AuditStatus::Success.to_string(), "SUCCESS");
}

#[test]
fn test_audit_status_display_failed() {
    assert_eq!(
        AuditStatus::Failed("error".to_string()).to_string(),
        "FAILED: error"
    );
}

#[test]
fn test_audit_status_display_pqc_failed() {
    let s = AuditStatus::PqcEncryptionFailed("kdf failed".to_string()).to_string();
    assert!(s.contains("FAILED:"));
    assert!(s.contains("PQC_ENCRYPTION_FAILED"));
}

#[test]
fn test_audit_status_display_integrity_failure() {
    let s = AuditStatus::IntegrityFailure("hash mismatch".to_string()).to_string();
    assert_eq!(s, "INTEGRITY_FAILURE: hash mismatch");
}

#[test]
fn test_audit_status_serialize_round_trip_via_json() {
    // Full round-trip through JSON serialization (as used in audit logs).
    let status = AuditStatus::Failed("disk full".to_string());
    let json = serde_json::to_value(&status).expect("serialise");
    let back: AuditStatus = serde_json::from_value(json).expect("deserialise");
    assert_eq!(status, back);
}

// =============================================================================
// MerkleTree — Edge cases
// =============================================================================

use llm_secure_cli::security::merkle::MerkleTree;

#[test]
fn test_merkle_tree_empty_leaves_produces_zero_root() {
    // Empty tree — this is a real code path:
    // if a session has zero audit entries, Merkle anchoring should not panic.
    let tree = MerkleTree::new(vec![]);
    assert_eq!(tree.root_hex, "0".repeat(64));
    assert!(tree.leaves.is_empty());
}

#[test]
fn test_merkle_tree_single_leaf_root_is_its_hash() {
    // MerkleTree expects pre-hashed leaves (64-char hex strings).
    // With a single leaf, the while loop never runs, so root = leaf.
    use sha2::{Digest, Sha256};
    let leaf_hash = llm_secure_cli::utils::hex_encode(Sha256::digest(b"test data"));
    assert_eq!(leaf_hash.len(), 64, "Hash must be 64 hex chars");
    let tree = MerkleTree::new(vec![leaf_hash.clone()]);
    assert_eq!(tree.root_hex, leaf_hash, "Single leaf's root must be the leaf itself");
}

#[test]
fn test_merkle_tree_two_leaves() {
    let leaf1 = "a".repeat(64);
    let leaf2 = "b".repeat(64);
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(leaf1.as_bytes());
    hasher.update(leaf2.as_bytes());
    let expected = llm_secure_cli::utils::hex_encode(hasher.finalize());
    let tree = MerkleTree::new(vec![leaf1, leaf2]);
    assert_eq!(tree.root_hex, expected);
}

#[test]
fn test_merkle_tree_odd_leaves_duplicates_last() {
    let leaf1 = "x".repeat(64);
    let leaf2 = "y".repeat(64);
    let leaf3 = "z".repeat(64);
    use sha2::{Digest, Sha256};
    // Level 1: hash(x, y), hash(z, z)
    let mut h1 = Sha256::new();
    h1.update(leaf1.as_bytes());
    h1.update(leaf2.as_bytes());
    let h1 = llm_secure_cli::utils::hex_encode(h1.finalize());
    let mut h2 = Sha256::new();
    h2.update(leaf3.as_bytes());
    h2.update(leaf3.as_bytes());
    let h2 = llm_secure_cli::utils::hex_encode(h2.finalize());
    // Level 2: hash(h1, h2)
    let mut root_h = Sha256::new();
    root_h.update(h1.as_bytes());
    root_h.update(h2.as_bytes());
    let expected_root = llm_secure_cli::utils::hex_encode(root_h.finalize());
    let tree = MerkleTree::new(vec![leaf1, leaf2, leaf3]);
    assert_eq!(tree.root_hex, expected_root);
}

// =============================================================================
// PQCVariant — String parsing (critical for config loading)
// =============================================================================

use llm_secure_cli::security::pqc::PQCVariant;
use std::str::FromStr;

#[test]
fn test_pqc_variant_from_str_standard_names() {
    assert_eq!(PQCVariant::from_str("ML-DSA-44").unwrap(), PQCVariant::MLDSA44);
    assert_eq!(PQCVariant::from_str("ML-DSA-65").unwrap(), PQCVariant::MLDSA65);
    assert_eq!(PQCVariant::from_str("ML-DSA-87").unwrap(), PQCVariant::MLDSA87);
}

#[test]
fn test_pqc_variant_from_str_compact_names() {
    // The code also handles compact forms (without hyphens)
    assert_eq!(PQCVariant::from_str("MLDSA44").unwrap(), PQCVariant::MLDSA44);
    assert_eq!(PQCVariant::from_str("MLDSA65").unwrap(), PQCVariant::MLDSA65);
    assert_eq!(PQCVariant::from_str("MLDSA87").unwrap(), PQCVariant::MLDSA87);
}

#[test]
fn test_pqc_variant_from_str_case_insensitive() {
    assert_eq!(PQCVariant::from_str("ml-dsa-44").unwrap(), PQCVariant::MLDSA44);
    assert_eq!(PQCVariant::from_str("mldsa87").unwrap(), PQCVariant::MLDSA87);
}

#[test]
fn test_pqc_variant_from_str_invalid() {
    assert!(PQCVariant::from_str("ML-DSA-128").is_err());
    assert!(PQCVariant::from_str("RSA-2048").is_err());
    assert!(PQCVariant::from_str("").is_err());
}

#[test]
fn test_pqc_variant_to_str() {
    assert_eq!(PQCVariant::MLDSA44.to_str(), "ML-DSA-44");
    assert_eq!(PQCVariant::MLDSA65.to_str(), "ML-DSA-65");
    assert_eq!(PQCVariant::MLDSA87.to_str(), "ML-DSA-87");
}

// =============================================================================
// SecurityContext::gather() — Security judgment input
// =============================================================================

use llm_secure_cli::security::policy::SecurityContext;

#[test]
fn test_security_context_has_required_fields() {
    let ctx = SecurityContext::gather("high");
    assert!(!ctx.os.is_empty(), "OS must be set");
    assert!(!ctx.user.is_empty(), "User must be set");
    assert!(!ctx.current_dir.is_empty(), "Current dir must be set");
    assert_eq!(ctx.security_level, "high");
    // container_mode and is_git_repo are environment-dependent; just verify they exist
    let _ = ctx.container_mode;
    let _ = ctx.is_git_repo;
}

#[test]
fn test_security_context_serializes_to_json() {
    let ctx = SecurityContext::gather("standard");
    let json = serde_json::to_value(&ctx).expect("Should serialise to JSON");
    assert_eq!(json["security_level"], "standard");
    assert!(json.get("os").and_then(|v| v.as_str()).is_some());
    assert!(json.get("user").and_then(|v| v.as_str()).is_some());
    assert!(json.get("current_dir").and_then(|v| v.as_str()).is_some());
    assert!(json.get("container_mode").and_then(|v| v.as_bool()).is_some());
    assert!(json.get("is_git_repo").and_then(|v| v.as_bool()).is_some());
}

// =============================================================================
// PathValidator — Symlink escape attacks
// =============================================================================
//
// These tests create real symlinks and attempt path traversal through them.
// They are the closest thing to an integration test for the path validator.

use llm_secure_cli::config::models::SecurityConfig;
use llm_secure_cli::security::path_validator::validate_path;
use tempfile::tempdir;

#[test]
fn test_path_validation_rejects_symlink_escape() {
    let dir = tempdir().expect("tempdir");
    let allowed = dir.path().join("allowed");
    std::fs::create_dir(&allowed).expect("create allowed dir");
    let outside = dir.path().join("outside");
    std::fs::create_dir(&outside).expect("create outside dir");
    let secret = outside.join("secret.txt");
    std::fs::write(&secret, "classified").expect("write secret");

    // Create symlink: allowed/link -> ../outside
    #[cfg(unix)]
    {
        let link = allowed.join("link");
        std::os::unix::fs::symlink(&outside, &link).expect("symlink");

        let config = SecurityConfig {
            allowed_paths: vec![allowed.to_string_lossy().to_string()],
            ..Default::default()
        };

        // Try to read secret.txt through the symlink
        let result = validate_path(
            link.join("secret.txt").to_str().unwrap(),
            &config,
        );
        // The symlink resolves to outside/ which is NOT in allowed_paths,
        // so this must be rejected.
        assert!(result.is_err(), "Symlink escape must be blocked");
    }
}

#[test]
fn test_path_validation_rejects_double_symlink_escape() {
    let dir = tempdir().expect("tempdir");
    let allowed = dir.path().join("allowed");
    std::fs::create_dir(&allowed).expect("create allowed");
    let intermediate = dir.path().join("intermediate");
    std::fs::create_dir(&intermediate).expect("create intermediate");
    let outside = dir.path().join("outside");
    std::fs::create_dir(&outside).expect("create outside");

    #[cfg(unix)]
    {
        // Chain: allowed/link1 -> ../intermediate, intermediate/link2 -> ../outside
        let link1 = allowed.join("link1");
        let link2 = intermediate.join("link2");
        std::os::unix::fs::symlink(&intermediate, &link1).expect("symlink1");
        std::os::unix::fs::symlink(&outside, &link2).expect("symlink2");

        let config = SecurityConfig {
            allowed_paths: vec![allowed.to_string_lossy().to_string()],
            ..Default::default()
        };

        let result = validate_path(
            link1.join("link2").to_str().unwrap(),
            &config,
        );
        assert!(result.is_err(), "Double symlink escape must be blocked");
    }
}

#[test]
fn test_path_validation_allows_symlink_within_boundary() {
    let dir = tempdir().expect("tempdir");
    let allowed = dir.path().join("allowed");
    std::fs::create_dir(&allowed).expect("create allowed");
    let sub = allowed.join("sub");
    std::fs::create_dir(&sub).expect("create sub");
    let target = allowed.join("target");
    std::fs::write(&target, "data").expect("write target");

    #[cfg(unix)]
    {
        let link = sub.join("link");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");

        let config = SecurityConfig {
            allowed_paths: vec![allowed.to_string_lossy().to_string()],
            ..Default::default()
        };

        // Symlink points to another file within the same allowed directory
        let result = validate_path(
            link.to_str().unwrap(),
            &config,
        );
        assert!(result.is_ok(), "Symlink within allowed boundary must pass");
    }
}

#[test]
fn test_path_validation_accepts_deeply_nested_path() {
    let dir = tempdir().expect("tempdir");
    let deep = dir.path().join("a").join("b").join("c").join("d");
    std::fs::create_dir_all(&deep).expect("create deep dir");

    let config = SecurityConfig {
        allowed_paths: vec![dir.path().to_string_lossy().to_string()],
        ..Default::default()
    };

    let result = validate_path(
        deep.to_str().unwrap(),
        &config,
    );
    assert!(result.is_ok(), "Deeply nested path within allowed dir must pass");
}

// =============================================================================
//
// The edit_file tool is high-risk (modifies actual files on disk).
// It must handle multiline content, trailing newlines, and edge cases
// without corrupting data.

#[test]
fn test_pqc_agility_manager_all_levels_map_correctly() {
    use llm_secure_cli::config::models::{AppConfig, SecurityLevel};
    use llm_secure_cli::security::pqc::{PQCAgilityManager, PQCVariant};

    let mut config = AppConfig::default();
    config.security.security_level = SecurityLevel::Standard;
    config.security.dual_llm_verification = Some(true);

    // execute_python is hardcoded as high-risk → ML-DSA-87
    let level = PQCAgilityManager::get_required_level(&config, "execute_python", None);
    assert_eq!(level, PQCVariant::MLDSA87, "High-risk tools must get ML-DSA-87");

    // list_files is low-risk → ML-DSA-44 (in Standard mode)
    let level = PQCAgilityManager::get_required_level(&config, "list_files", None);
    assert_eq!(level, PQCVariant::MLDSA44, "Low-risk tools must get ML-DSA-44");

    // With security_level=High, low-risk tools escalate to Medium → ML-DSA-65
    config.security.security_level = SecurityLevel::High;
    let level = PQCAgilityManager::get_required_level(&config, "list_files", None);
    assert_eq!(level, PQCVariant::MLDSA65, "Low-risk in High security must get ML-DSA-65");
}