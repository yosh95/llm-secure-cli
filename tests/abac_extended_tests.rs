use llm_secure_cli::config::models::AppConfig;
use llm_secure_cli::security::abac::AbacEngine;
use llm_secure_cli::security::policy::EvaluationContext;
use serde_json::json;

#[test]
fn test_abac_prefix_matching() {
    let config_content = r#"
[security]
[[security.abac_rules]]
name = "Allow prefix path"
effect = "allow"
match_attributes = { "env.cwd" = "prefix:/home/user/project" }
"#;
    let config: AppConfig = toml::from_str(config_content).unwrap();

    // 1. Matches prefix
    let mut ctx = EvaluationContext::default();
    ctx.set_attribute("env.cwd", json!("/home/user/project/subdir"));
    assert_eq!(
        AbacEngine::evaluate_with_config(&config, &ctx),
        Some("allow".to_string())
    );

    // 2. Exact match still works with prefix
    let mut ctx = EvaluationContext::default();
    ctx.set_attribute("env.cwd", json!("/home/user/project"));
    assert_eq!(
        AbacEngine::evaluate_with_config(&config, &ctx),
        Some("allow".to_string())
    );

    // 3. Does not match
    let mut ctx = EvaluationContext::default();
    ctx.set_attribute("env.cwd", json!("/var/log"));
    assert_eq!(AbacEngine::evaluate_with_config(&config, &ctx), None);
}

#[test]
fn test_abac_array_contains() {
    let config_content = r#"
[security]
[[security.abac_rules]]
name = "Allow if group matches"
effect = "allow"
match_attributes = { "subject.groups" = "admin" }
"#;
    let config: AppConfig = toml::from_str(config_content).unwrap();

    // Context attribute is an array containing "admin"
    let mut ctx = EvaluationContext::default();
    ctx.set_attribute("subject.groups", json!(["user", "admin", "tester"]));
    assert_eq!(
        AbacEngine::evaluate_with_config(&config, &ctx),
        Some("allow".to_string())
    );

    // Does not contain
    let mut ctx = EvaluationContext::default();
    ctx.set_attribute("subject.groups", json!(["user", "tester"]));
    assert_eq!(AbacEngine::evaluate_with_config(&config, &ctx), None);
}

#[test]
fn test_abac_subset_match() {
    let config_content = r#"
[security]
[[security.abac_rules]]
name = "Allow if groups are subset"
effect = "allow"
match_attributes = { "subject.groups" = ["admin", "security"] }
"#;
    let config: AppConfig = toml::from_str(config_content).unwrap();

    // Context has both required groups
    let mut ctx = EvaluationContext::default();
    ctx.set_attribute(
        "subject.groups",
        json!(["user", "security", "admin", "dev"]),
    );
    assert_eq!(
        AbacEngine::evaluate_with_config(&config, &ctx),
        Some("allow".to_string())
    );

    // Context is missing one
    let mut ctx = EvaluationContext::default();
    ctx.set_attribute("subject.groups", json!(["user", "admin", "dev"]));
    assert_eq!(AbacEngine::evaluate_with_config(&config, &ctx), None);
}
