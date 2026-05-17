use llm_secure_cli::config::models::SecurityConfig;
use llm_secure_cli::security::cass::{CASSOrchestrator, RiskLevel};
use serde_json::json;

#[test]
fn test_evaluate_risk_baseline() {
    let config = SecurityConfig {
        security_level: "standard".to_string(),
        high_risk_tools: vec!["edit_file".to_string()],
        medium_risk_tools: vec!["read_file".to_string()],
        dual_llm_verification: Some(true),
        ..Default::default()
    };

    let orchestrator = CASSOrchestrator;

    assert_eq!(
        orchestrator.evaluate_risk("edit_file", None, &config),
        RiskLevel::High
    );
    assert_eq!(
        orchestrator.evaluate_risk("read_file", None, &config),
        RiskLevel::Medium
    );
    assert_eq!(
        orchestrator.evaluate_risk("list_files", None, &config),
        RiskLevel::Low
    );
}

#[test]
fn test_evaluate_risk_critical_escalation_no_dual_llm() {
    let config = SecurityConfig {
        security_level: "standard".to_string(),
        high_risk_tools: vec!["edit_file".to_string()],
        dual_llm_verification: Some(false),
        ..Default::default()
    };

    let orchestrator = CASSOrchestrator;

    // High risk tool becomes Critical when Dual LLM is off
    assert_eq!(
        orchestrator.evaluate_risk("edit_file", None, &config),
        RiskLevel::Critical
    );

    // Command execution is always at least High, so moves to Critical
    assert_eq!(
        orchestrator.evaluate_risk("execute_command", None, &config),
        RiskLevel::Critical
    );
}

#[test]
fn test_evaluate_risk_dynamic_escalation() {
    let config = SecurityConfig {
        security_level: "standard".to_string(),
        medium_risk_tools: vec!["read_file".to_string()],
        scaling_patterns: vec!["/etc/shadow".to_string()],
        dual_llm_verification: Some(true),
        ..Default::default()
    };

    let orchestrator = CASSOrchestrator;

    // Medium tool stays Medium usually
    assert_eq!(
        orchestrator.evaluate_risk("read_file", Some(&json!({"path": "normal.txt"})), &config),
        RiskLevel::Medium
    );

    // Medium tool escalates to High if sensitive pattern is found
    assert_eq!(
        orchestrator.evaluate_risk("read_file", Some(&json!({"path": "/etc/shadow"})), &config),
        RiskLevel::High
    );
}

#[test]
fn test_evaluate_risk_security_level_high() {
    let config = SecurityConfig {
        security_level: "high".to_string(),
        dual_llm_verification: Some(true),
        ..Default::default()
    };

    let orchestrator = CASSOrchestrator;

    // In 'high' security mode, Low risk tools escalate to Medium
    assert_eq!(
        orchestrator.evaluate_risk("list_files", None, &config),
        RiskLevel::Medium
    );
}

#[test]
fn test_get_security_requirements_returns_correct_pqc_levels() {
    let config = SecurityConfig {
        security_level: "standard".to_string(),
        high_risk_tools: vec!["edit_file".to_string()],
        dual_llm_verification: Some(true),
        ..Default::default()
    };

    let orchestrator = CASSOrchestrator;

    // Critical: execute_command without dual llm
    let critical_config = SecurityConfig {
        security_level: "standard".to_string(),
        dual_llm_verification: Some(false),
        ..Default::default()
    };
    let posture = orchestrator.get_security_requirements("execute_command", None, &critical_config);
    assert!(posture.require_pqc_signature);
    assert_eq!(posture.pqc_variant, "ML-DSA-87");
    assert!(posture.require_pqc_audit_encryption);
    assert!(posture.require_dual_llm_verification);

    // High: edit_file (high risk tool with dual llm on)
    let posture = orchestrator.get_security_requirements("edit_file", None, &config);
    assert!(posture.require_pqc_signature);
    assert_eq!(posture.pqc_variant, "ML-DSA-87");
    assert!(posture.require_pqc_audit_encryption);
    assert!(posture.require_dual_llm_verification);

    // Medium
    let medium_config = SecurityConfig {
        security_level: "standard".to_string(),
        medium_risk_tools: vec!["read_file".to_string()],
        dual_llm_verification: Some(true),
        ..Default::default()
    };
    let posture = orchestrator.get_security_requirements("read_file", None, &medium_config);
    assert!(posture.require_pqc_signature);
    assert_eq!(posture.pqc_variant, "ML-DSA-65");
    assert!(!posture.require_pqc_audit_encryption);
    assert!(!posture.require_dual_llm_verification);

    // Low
    let posture = orchestrator.get_security_requirements("list_files", None, &config);
    assert!(posture.require_pqc_signature);
    assert_eq!(posture.pqc_variant, "ML-DSA-44");
    assert!(!posture.require_pqc_audit_encryption);
    assert!(!posture.require_dual_llm_verification);
}

#[test]
fn test_evaluate_risk_blocked_paths_escalation() {
    let config = SecurityConfig {
        security_level: "standard".to_string(),
        blocked_paths: vec!["/etc/passwd".to_string()],
        dual_llm_verification: Some(true),
        ..Default::default()
    };

    let orchestrator = CASSOrchestrator;

    // A low-risk tool accessing a blocked path must escalate
    let risk =
        orchestrator.evaluate_risk("read_file", Some(&json!({"path": "/etc/passwd"})), &config);
    assert_eq!(risk, RiskLevel::High);
}
