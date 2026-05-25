use llm_secure_cli::config::models::SecurityConfig;
use llm_secure_cli::security::cass::{CASSOrchestrator, RiskLevel};
use llm_secure_cli::security::merkle::MerkleTree;
use serde_json::json;

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

    // Risk-level-based scaling is discontinued.
    // CASSOrchestrator::evaluate_risk always returns Low.
    let level = CASSOrchestrator::evaluate_risk("read_file", None, &config);
    assert_eq!(level, RiskLevel::Low);

    let level = CASSOrchestrator::evaluate_risk("execute_python", None, &config);
    assert_eq!(level, RiskLevel::Low);

    let config_with_patterns = SecurityConfig {
        scaling_patterns: vec!["/etc/shadow".to_string()],
        ..SecurityConfig::default()
    };

    let level = CASSOrchestrator::evaluate_risk(
        "read_file",
        Some(&json!({"path": "/etc/shadow"})),
        &config_with_patterns,
    );
    assert_eq!(level, RiskLevel::Low);
}
