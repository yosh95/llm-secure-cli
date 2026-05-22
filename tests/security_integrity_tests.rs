use llm_secure_cli::config::models::SecurityConfig;
use llm_secure_cli::security::cass::{CASSOrchestrator, RiskLevel};
use llm_secure_cli::security::merkle::MerkleTree;
use llm_secure_cli::security::path_validator::validate_path;
use serde_json::json;
use std::fs;
use tempfile::tempdir;

#[test]
fn test_path_validator_traversal_and_links() {
    let tmp = tempdir().expect("Failed to create temp dir");
    let base_path = tmp
        .path()
        .canonicalize()
        .expect("Failed to canonicalize path");

    // Create physical environment
    let safe_dir = base_path.join("safe");
    fs::create_dir(&safe_dir).expect("Failed to create safe dir");
    let safe_file = safe_dir.join("data.txt");
    fs::write(&safe_file, "secret").expect("Failed to write safe file");

    let sensitive_file = base_path.join("sensitive.key");
    fs::write(&sensitive_file, "private_key").expect("Failed to write sensitive file");

    // Mock config
    let config = SecurityConfig {
        allowed_paths: vec![safe_dir.to_string_lossy().to_string()],
        ..SecurityConfig::default()
    };

    // 1. Basic valid path
    assert!(validate_path(&safe_file.to_string_lossy(), &config).is_ok());

    // 2. Test classic traversal (../)
    let traversal_path = safe_dir.join("../sensitive.key");
    // Should now return Err because it resolves to a path outside safe_dir
    assert!(validate_path(&traversal_path.to_string_lossy(), &config).is_err());

    // 3. Test Symbolic Link Attack
    #[cfg(unix)]
    {
        let symlink_path = safe_dir.join("leak.txt");
        // Create a symlink inside safe_dir pointing to sensitive_file outside
        let _ = std::os::unix::fs::symlink(&sensitive_file, &symlink_path);

        // validate_path resolves symlinks - it should now fail because the actual target is outside safe_dir
        assert!(validate_path(&symlink_path.to_string_lossy(), &config).is_err());
    }
}

#[test]
fn test_merkle_audit_integrity() {
    let logs = vec![
        "session_start".to_string(),
        "tool_call: ls".to_string(),
        "tool_result: file.txt".to_string(),
    ];

    // 1. Root hash generation
    let tree = MerkleTree::new(logs.clone());
    let root_before = tree.root_hex.clone();
    assert_ne!(root_before, "0".repeat(64));

    // 2. Tamper test
    let mut tampered_logs = logs.clone();
    tampered_logs[1] = "tool_call: rm -rf /".to_string();
    let tree_tampered = MerkleTree::new(tampered_logs);

    assert_ne!(
        root_before, tree_tampered.root_hex,
        "Tampered log must result in different hash"
    );
}

#[test]
fn test_cass_risk_scaling() {
    let config = SecurityConfig::default();

    // 1. Test Low Risk Tool (default)
    let level_low = CASSOrchestrator::evaluate_risk("read_file", None, &config);
    assert!(level_low <= RiskLevel::Medium);

    // 2. Test Critical Risk Tool
    let level_crit = CASSOrchestrator::evaluate_risk("execute_python", None, &config);
    assert_eq!(level_crit, RiskLevel::Critical);

    // 3. Test Argument-based escalation
    let config_with_patterns = SecurityConfig {
        scaling_patterns: vec!["/etc/shadow".to_string()],
        ..SecurityConfig::default()
    };

    let sensitive_read = CASSOrchestrator::evaluate_risk(
        "read_file",
        Some(&json!({"path": "/etc/shadow"})),
        &config_with_patterns,
    );

    assert!(sensitive_read >= RiskLevel::High);
}
