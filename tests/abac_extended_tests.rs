use llm_secure_cli::config::CONFIG_MANAGER;
use llm_secure_cli::security::abac::AbacEngine;
use llm_secure_cli::security::policy::EvaluationContext;
use serde_json::json;
use std::env;
use std::fs;
use tempfile::tempdir;

#[test]
fn test_abac_prefix_matching() {
    let dir = tempdir().unwrap();
    let original_dir = env::current_dir().unwrap();
    env::set_current_dir(dir.path()).unwrap();

    let config_content = r#"
[security]
[[security.abac_rules]]
name = "Allow prefix path"
effect = "allow"
match_attributes = { "env.cwd" = "prefix:/home/user/project" }
"#;
    fs::write(dir.path().join("config.toml"), config_content).unwrap();
    CONFIG_MANAGER.reload();

    // 1. Matches prefix
    let mut ctx = EvaluationContext::default();
    ctx.set_attribute("env.cwd", json!("/home/user/project/subdir"));
    assert_eq!(AbacEngine::evaluate(&ctx), Some("allow".to_string()));

    // 2. Exact match still works with prefix
    let mut ctx = EvaluationContext::default();
    ctx.set_attribute("env.cwd", json!("/home/user/project"));
    assert_eq!(AbacEngine::evaluate(&ctx), Some("allow".to_string()));

    // 3. Does not match
    let mut ctx = EvaluationContext::default();
    ctx.set_attribute("env.cwd", json!("/var/log"));
    assert_eq!(AbacEngine::evaluate(&ctx), None);

    env::set_current_dir(original_dir).unwrap();
}

#[test]
fn test_abac_array_contains() {
    let dir = tempdir().unwrap();
    let original_dir = env::current_dir().unwrap();
    env::set_current_dir(dir.path()).unwrap();

    let config_content = r#"
[security]
[[security.abac_rules]]
name = "Allow if group matches"
effect = "allow"
match_attributes = { "subject.groups" = "admin" }
"#;
    fs::write(dir.path().join("config.toml"), config_content).unwrap();
    CONFIG_MANAGER.reload();

    // Context attribute is an array containing "admin"
    let mut ctx = EvaluationContext::default();
    ctx.set_attribute("subject.groups", json!(["user", "admin", "tester"]));
    assert_eq!(AbacEngine::evaluate(&ctx), Some("allow".to_string()));

    // Does not contain
    let mut ctx = EvaluationContext::default();
    ctx.set_attribute("subject.groups", json!(["user", "tester"]));
    assert_eq!(AbacEngine::evaluate(&ctx), None);

    env::set_current_dir(original_dir).unwrap();
}

#[test]
fn test_abac_subset_match() {
    let dir = tempdir().unwrap();
    let original_dir = env::current_dir().unwrap();
    env::set_current_dir(dir.path()).unwrap();

    let config_content = r#"
[security]
[[security.abac_rules]]
name = "Allow if groups are subset"
effect = "allow"
match_attributes = { "subject.groups" = ["admin", "security"] }
"#;
    fs::write(dir.path().join("config.toml"), config_content).unwrap();
    CONFIG_MANAGER.reload();

    // Context has both required groups
    let mut ctx = EvaluationContext::default();
    ctx.set_attribute(
        "subject.groups",
        json!(["user", "security", "admin", "dev"]),
    );
    assert_eq!(AbacEngine::evaluate(&ctx), Some("allow".to_string()));

    // Context is missing one
    let mut ctx = EvaluationContext::default();
    ctx.set_attribute("subject.groups", json!(["user", "admin", "dev"]));
    assert_eq!(AbacEngine::evaluate(&ctx), None);

    env::set_current_dir(original_dir).unwrap();
}
