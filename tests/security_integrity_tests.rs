#![allow(clippy::unwrap_used, clippy::expect_used)]
use llm_secure_cli::security::merkle::MerkleTree;

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
