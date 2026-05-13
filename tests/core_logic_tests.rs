use llm_secure_cli::config::models::SecurityConfig;
use llm_secure_cli::security::static_analyzer::StaticAnalyzer;
use llm_secure_cli::security::validate_tool_call;
use serde_json::json;

#[test]
fn test_static_analyzer_obviously_malicious() {
    // Current StaticAnalyzer only blocks null bytes
    assert!(StaticAnalyzer::is_obviously_malicious("safe command\0"));
    assert!(!StaticAnalyzer::is_obviously_malicious("safe command"));
}

#[test]
fn test_validate_tool_call_path_blocks() {
    // Testing validate_tool_call which is now a placeholder (Phase 1 simplicity)
    let config = SecurityConfig::default();

    // 1. Path Traversal
    let mut args = serde_json::Map::new();
    args.insert("path".to_string(), json!("../../../etc/passwd"));
    let result = validate_tool_call("read_file", &args, &config);
    // Now returns Ok, as semantic verification handles this in Phase 3
    assert!(result.is_ok());

    // 2. Safe path
    let mut args = serde_json::Map::new();
    args.insert("path".to_string(), json!("README.md"));
    let result = validate_tool_call("read_file", &args, &config);
    assert!(result.is_ok());
}

#[test]
fn test_validate_tool_call_static_analysis() {
    // Testing Phase 1 bypass (now delegated to Dual LLM)
    let config = SecurityConfig::default();
    let mut args = serde_json::Map::new();
    args.insert("command".to_string(), json!("ls\0"));
    let result = validate_tool_call("execute_command", &args, &config);
    // Now returns Ok
    assert!(result.is_ok());
}

#[test]
fn test_merkle_tree_robustness() {
    use llm_secure_cli::security::merkle::MerkleTree;

    // Case 1: Empty tree
    let tree = MerkleTree::new(vec![]);
    assert_eq!(tree.root_hex, "0".repeat(64));

    // Case 2: Single node
    let leaf = "a".repeat(64);
    let tree = MerkleTree::new(vec![leaf.clone()]);
    assert_eq!(tree.root_hex, leaf);

    // Case 3: Balanced tree (2 nodes)
    let leaves = vec!["a".repeat(64), "b".repeat(64)];
    let tree = MerkleTree::new(leaves.clone());
    assert_ne!(tree.root_hex, leaves[0]);
    assert_ne!(tree.root_hex, leaves[1]);
}
