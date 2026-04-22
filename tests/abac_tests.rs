use llm_secure_cli::config::CONFIG_MANAGER;
use llm_secure_cli::security::abac::AbacEngine;
use llm_secure_cli::security::policy::EvaluationContext;
use serde_json::json;
use std::env;
use std::fs;
use tempfile::tempdir;

#[test]
fn test_abac_rule_matching() {
    let dir = tempdir().unwrap();
    let original_dir = env::current_dir().unwrap();
    env::set_current_dir(dir.path()).unwrap();

    // Create a dummy config.toml with ABAC rules
    let config_content = r#"
[security]
[[security.abac_rules]]
name = "Allow developer on main branch"
effect = "allow"
match_attributes = { "subject.role" = "developer", "env.git_branch" = "main" }

[[security.abac_rules]]
name = "Deny production access for interns"
effect = "deny"
match_attributes = { "subject.role" = "intern", "env.target" = "production" }
"#;
    fs::write(dir.path().join("config.toml"), config_content).unwrap();

    // Reload config
    CONFIG_MANAGER.reload();

    // 1. Success case: Developer on main branch
    let mut ctx = EvaluationContext::default();
    ctx.set_attribute("subject.role", json!("developer"));
    ctx.set_attribute("env.git_branch", json!("main"));

    let result = AbacEngine::evaluate(&ctx);
    assert_eq!(result, Some("allow".to_string()));

    // 2. Deny case: Intern on production
    let mut ctx = EvaluationContext::default();
    ctx.set_attribute("subject.role", json!("intern"));
    ctx.set_attribute("env.target", json!("production"));

    let result = AbacEngine::evaluate(&ctx);
    assert_eq!(result, Some("deny".to_string()));

    // 3. No match: Developer on dev branch (doesn't match first rule because of branch, doesn't match second)
    let mut ctx = EvaluationContext::default();
    ctx.set_attribute("subject.role", json!("developer"));
    ctx.set_attribute("env.git_branch", json!("dev"));

    let result = AbacEngine::evaluate(&ctx);
    assert_eq!(result, None);

    // 4. Missing attribute: Only role developer, missing branch
    let mut ctx = EvaluationContext::default();
    ctx.set_attribute("subject.role", json!("developer"));

    let result = AbacEngine::evaluate(&ctx);
    assert_eq!(result, None);

    env::set_current_dir(original_dir).unwrap();
}

#[test]
fn test_evaluation_context_system_attributes() {
    use llm_secure_cli::security::policy::EvaluationContext;
    let ctx = EvaluationContext::new();

    // Check for some standard attributes
    assert!(ctx.get_attribute("subject.id").is_some());
    assert!(ctx.get_attribute("env.os").is_some());
    assert!(ctx.get_attribute("env.cwd").is_some());

    // Check if OS attribute is a string
    let os = ctx.get_attribute("env.os").unwrap();
    assert!(os.is_string());
}

#[test]
fn test_abac_array_attributes() {
    let dir = tempdir().unwrap();
    let original_dir = env::current_dir().unwrap();
    env::set_current_dir(dir.path()).unwrap();

    let config_content = r#"
[security]
[[security.abac_rules]]
name = "Allow specific roles"
effect = "allow"
match_attributes = { "subject.groups" = ["admin", "security"] }
"#;
    fs::write(dir.path().join("config.toml"), config_content).unwrap();
    CONFIG_MANAGER.reload();

    // Exact match
    let mut ctx = EvaluationContext::default();
    ctx.set_attribute("subject.groups", json!(["admin", "security"]));
    assert_eq!(AbacEngine::evaluate(&ctx), Some("allow".to_string()));

    // Partial match (should fail with current implementation)
    let mut ctx = EvaluationContext::default();
    ctx.set_attribute("subject.groups", json!(["admin"]));
    assert_eq!(AbacEngine::evaluate(&ctx), None);

    // Different order (should fail for exact array match)
    let mut ctx = EvaluationContext::default();
    ctx.set_attribute("subject.groups", json!(["security", "admin"]));
    assert_eq!(AbacEngine::evaluate(&ctx), None);

    env::set_current_dir(original_dir).unwrap();
}

#[test]
fn test_policy_engine_integration_with_abac() {
    let dir = tempdir().unwrap();
    let original_dir = env::current_dir().unwrap();
    env::set_current_dir(dir.path()).unwrap();

    let config_content = r#"
[security]
[[security.abac_rules]]
name = "Deny all tool execution for untrusted users"
effect = "deny"
match_attributes = { "subject.trust_level" = "untrusted" }
"#;
    fs::write(dir.path().join("config.toml"), config_content).unwrap();
    CONFIG_MANAGER.reload();

    use llm_secure_cli::security::policy::{EvaluationContext, PolicyEngine};
    let engine = PolicyEngine;
    let mut ctx = EvaluationContext::default();
    ctx.set_attribute("subject.trust_level", json!("untrusted"));

    let args = serde_json::Map::new();
    let allowed = engine.evaluate("some_tool", &args, &ctx);

    assert!(!allowed, "Policy should be denied by ABAC rule");

    // Change trust level to trusted
    let mut ctx = EvaluationContext::default();
    ctx.set_attribute("subject.trust_level", json!("trusted"));
    let allowed = engine.evaluate("some_tool", &args, &ctx);

    assert!(
        allowed,
        "Policy should be allowed when no ABAC deny rule matches"
    );

    env::set_current_dir(original_dir).unwrap();
}

#[test]
fn test_abac_numeric_and_bool_attributes() {
    let dir = tempdir().unwrap();
    let original_dir = env::current_dir().unwrap();
    env::set_current_dir(dir.path()).unwrap();

    let config_content = r#"
[security]
[[security.abac_rules]]
name = "High risk if confidence low"
effect = "deny"
match_attributes = { "risk.score" = 90, "risk.verified" = false }
"#;
    fs::write(dir.path().join("config.toml"), config_content).unwrap();
    CONFIG_MANAGER.reload();

    // Match
    let mut ctx = EvaluationContext::default();
    ctx.set_attribute("risk.score", json!(90));
    ctx.set_attribute("risk.verified", json!(false));
    assert_eq!(AbacEngine::evaluate(&ctx), Some("deny".to_string()));

    // No match (different score)
    let mut ctx = EvaluationContext::default();
    ctx.set_attribute("risk.score", json!(80));
    ctx.set_attribute("risk.verified", json!(false));
    assert_eq!(AbacEngine::evaluate(&ctx), None);

    // No match (different bool)
    let mut ctx = EvaluationContext::default();
    ctx.set_attribute("risk.score", json!(90));
    ctx.set_attribute("risk.verified", json!(true));
    assert_eq!(AbacEngine::evaluate(&ctx), None);

    env::set_current_dir(original_dir).unwrap();
}
