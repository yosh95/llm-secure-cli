use llm_secure_cli::config::models::{SecurityConfig, SecurityLevel};
use llm_secure_cli::security::cass::{CASSOrchestrator, RiskLevel};
use serde_json::json;

#[test]
fn test_evaluate_risk_baseline() {
    let config = SecurityConfig {
        security_level: SecurityLevel::Standard,
        high_risk_tools: vec!["edit_file".to_string()],
        medium_risk_tools: vec!["read_file".to_string()],
        dual_llm_verification: Some(true),
        ..Default::default()
    };

    assert_eq!(
        CASSOrchestrator::evaluate_risk("edit_file", None, &config),
        RiskLevel::High
    );
    assert_eq!(
        CASSOrchestrator::evaluate_risk("read_file", None, &config),
        RiskLevel::Medium
    );
    assert_eq!(
        CASSOrchestrator::evaluate_risk("list_files", None, &config),
        RiskLevel::Low
    );
}

#[test]
fn test_evaluate_risk_critical_escalation_no_dual_llm() {
    let config = SecurityConfig {
        security_level: SecurityLevel::Standard,
        high_risk_tools: vec!["edit_file".to_string()],
        dual_llm_verification: Some(false),
        ..Default::default()
    };

    // High risk tool becomes Critical when Dual LLM is off
    assert_eq!(
        CASSOrchestrator::evaluate_risk("edit_file", None, &config),
        RiskLevel::Critical
    );

    // Command execution is always at least High, so moves to Critical
    assert_eq!(
        CASSOrchestrator::evaluate_risk("execute_python", None, &config),
        RiskLevel::Critical
    );
}

#[test]
fn test_evaluate_risk_dynamic_escalation() {
    let config = SecurityConfig {
        security_level: SecurityLevel::Standard,
        medium_risk_tools: vec!["read_file".to_string()],
        scaling_patterns: vec!["/etc/shadow".to_string()],
        dual_llm_verification: Some(true),
        ..Default::default()
    };

    // Medium tool stays Medium usually
    assert_eq!(
        CASSOrchestrator::evaluate_risk("read_file", Some(&json!({"path": "normal.txt"})), &config),
        RiskLevel::Medium
    );

    // Medium tool escalates to High if sensitive pattern is found
    assert_eq!(
        CASSOrchestrator::evaluate_risk(
            "read_file",
            Some(&json!({"path": "/etc/shadow"})),
            &config
        ),
        RiskLevel::High
    );
}

#[test]
fn test_evaluate_risk_security_level_high() {
    let config = SecurityConfig {
        security_level: SecurityLevel::High,
        dual_llm_verification: Some(true),
        ..Default::default()
    };

    // In 'high' security mode, Low risk tools escalate to Medium
    assert_eq!(
        CASSOrchestrator::evaluate_risk("list_files", None, &config),
        RiskLevel::Medium
    );
}

#[test]
fn test_get_security_requirements_returns_correct_pqc_levels() {
    let config = SecurityConfig {
        security_level: SecurityLevel::Standard,
        high_risk_tools: vec!["edit_file".to_string()],
        dual_llm_verification: Some(true),
        ..Default::default()
    };

    // Critical: execute_python without dual llm
    let critical_config = SecurityConfig {
        security_level: SecurityLevel::Standard,
        dual_llm_verification: Some(false),
        ..Default::default()
    };
    let posture =
        CASSOrchestrator::get_security_requirements("execute_python", None, &critical_config);
    assert!(posture.require_pqc_signature);
    assert_eq!(posture.pqc_variant, "ML-DSA-87");
    assert!(posture.require_pqc_audit_encryption);
    assert!(posture.require_dual_llm_verification);

    // High: edit_file (high risk tool with dual llm on)
    let posture = CASSOrchestrator::get_security_requirements("edit_file", None, &config);
    assert!(posture.require_pqc_signature);
    assert_eq!(posture.pqc_variant, "ML-DSA-87");
    assert!(posture.require_pqc_audit_encryption);
    assert!(posture.require_dual_llm_verification);

    // Medium
    let medium_config = SecurityConfig {
        security_level: SecurityLevel::Standard,
        medium_risk_tools: vec!["read_file".to_string()],
        dual_llm_verification: Some(true),
        ..Default::default()
    };
    let posture = CASSOrchestrator::get_security_requirements("read_file", None, &medium_config);
    assert!(posture.require_pqc_signature);
    assert_eq!(posture.pqc_variant, "ML-DSA-65");
    assert!(!posture.require_pqc_audit_encryption);
    assert!(!posture.require_dual_llm_verification);

    // Low
    let posture = CASSOrchestrator::get_security_requirements("list_files", None, &config);
    assert!(posture.require_pqc_signature);
    assert_eq!(posture.pqc_variant, "ML-DSA-44");
    assert!(!posture.require_pqc_audit_encryption);
    assert!(!posture.require_dual_llm_verification);
}

#[test]
fn test_evaluate_risk_blocked_paths_escalation() {
    let config = SecurityConfig {
        security_level: SecurityLevel::Standard,
        blocked_paths: vec!["/etc/passwd".to_string()],
        dual_llm_verification: Some(true),
        ..Default::default()
    };

    // A low-risk tool accessing a blocked path must escalate
    let risk = CASSOrchestrator::evaluate_risk(
        "read_file",
        Some(&json!({"path": "/etc/passwd"})),
        &config,
    );
    assert_eq!(risk, RiskLevel::High);
}

#[test]
fn test_evaluate_risk_scaling_patterns_case_insensitive() {
    let config = SecurityConfig {
        security_level: SecurityLevel::Standard,
        scaling_patterns: vec!["/etc/shadow".to_string()],
        dual_llm_verification: Some(true),
        ..Default::default()
    };

    // Case-insensitive matching via to_lowercase()
    let risk = CASSOrchestrator::evaluate_risk(
        "read_file",
        Some(&json!({"path": "/ETC/SHADOW"})),
        &config,
    );
    assert_eq!(risk, RiskLevel::High);
}

#[test]
fn test_evaluate_risk_unknown_tool_defaults_to_low() {
    let config = SecurityConfig {
        security_level: SecurityLevel::Standard,
        dual_llm_verification: Some(true),
        ..Default::default()
    };

    // An unknown tool not in any risk list defaults to Low
    let risk = CASSOrchestrator::evaluate_risk("completely_unknown_tool", None, &config);
    assert_eq!(risk, RiskLevel::Low);
}

#[test]
fn test_evaluate_risk_high_args_existing_high_tool_stays_high() {
    let config = SecurityConfig {
        security_level: SecurityLevel::Standard,
        high_risk_tools: vec!["edit_file".to_string()],
        blocked_paths: vec!["/etc/shadow".to_string()],
        dual_llm_verification: Some(true),
        ..Default::default()
    };

    // Already High tool with sensitive args stays High (doesn't overflow)
    let risk = CASSOrchestrator::evaluate_risk(
        "edit_file",
        Some(&json!({"path": "/etc/shadow"})),
        &config,
    );
    assert_eq!(risk, RiskLevel::High);
}

#[test]
fn test_evaluate_risk_high_args_on_already_medium_tool() {
    let config = SecurityConfig {
        security_level: SecurityLevel::Standard,
        medium_risk_tools: vec!["read_file".to_string()],
        scaling_patterns: vec!["/root/".to_string()],
        dual_llm_verification: Some(true),
        ..Default::default()
    };

    // Medium tool with sensitive pattern → escalates to High
    let risk = CASSOrchestrator::evaluate_risk(
        "read_file",
        Some(&json!({"path": "/root/.ssh/id_rsa"})),
        &config,
    );
    assert_eq!(risk, RiskLevel::High);
}

#[test]
fn test_evaluate_risk_dual_llm_enabled_prevents_critical() {
    let config = SecurityConfig {
        security_level: SecurityLevel::Standard,
        dual_llm_verification: Some(true),
        ..Default::default()
    };

    // execute_python is always High, and with dual_llm enabled it stays High
    let risk = CASSOrchestrator::evaluate_risk("execute_python", None, &config);
    assert_eq!(risk, RiskLevel::High);
}

#[test]
fn test_evaluate_risk_dual_llm_none_treated_as_disabled() {
    let config = SecurityConfig {
        security_level: SecurityLevel::Standard,
        high_risk_tools: vec!["edit_file".to_string()],
        dual_llm_verification: None,
        ..Default::default()
    };

    // dual_llm_verification: None → treated as disabled → Critical
    let risk = CASSOrchestrator::evaluate_risk("edit_file", None, &config);
    assert_eq!(risk, RiskLevel::Critical);
}

#[test]
fn test_evaluate_risk_multiple_blocked_paths_match() {
    let config = SecurityConfig {
        security_level: SecurityLevel::Standard,
        blocked_paths: vec![
            "/etc/passwd".to_string(),
            "/etc/shadow".to_string(),
            "/root/.ssh".to_string(),
        ],
        dual_llm_verification: Some(true),
        ..Default::default()
    };

    // Any one of the blocked paths triggers escalation
    let risk = CASSOrchestrator::evaluate_risk(
        "read_file",
        Some(&json!({"path": "/root/.ssh/authorized_keys"})),
        &config,
    );
    assert_eq!(risk, RiskLevel::High);
}

#[test]
fn test_evaluate_risk_complex_nested_args() {
    let config = SecurityConfig {
        security_level: SecurityLevel::Standard,
        scaling_patterns: vec!["DROP TABLE".to_string()],
        dual_llm_verification: Some(true),
        ..Default::default()
    };

    // The args are nested JSON; to_string() flattens it, and patterns are matched
    let risk = CASSOrchestrator::evaluate_risk(
        "execute_python",
        Some(&json!({
            "code": "import sqlite3; cursor.execute('DROP TABLE users')"
        })),
        &config,
    );
    // execute_python is already High (hardcoded); pattern check keeps it High
    assert_eq!(risk, RiskLevel::High);
}

#[test]
fn test_evaluate_risk_security_level_high_with_execute_python() {
    let config = SecurityConfig {
        security_level: SecurityLevel::High,
        dual_llm_verification: Some(true),
        ..Default::default()
    };

    // execute_python is always at least High; security_level=high doesn't escalate beyond
    let risk = CASSOrchestrator::evaluate_risk("execute_python", None, &config);
    assert!(risk >= RiskLevel::High);
}

#[test]
fn test_get_security_requirements_risk_level_ord() {
    // Verify RiskLevel ordering: Low < Medium < High < Critical
    assert!(RiskLevel::Low < RiskLevel::Medium);
    assert!(RiskLevel::Medium < RiskLevel::High);
    assert!(RiskLevel::High < RiskLevel::Critical);
}

#[test]
fn test_cass_orchestrator_is_send_sync() {
    // CASSOrchestrator is a unit struct; verify it is Send + Sync for use
    // across async task boundaries.
    fn _assert_send_sync<T: Send + Sync>(_: &T) {}
    let orchestrator = llm_secure_cli::security::cass::CASSOrchestrator;
    _assert_send_sync(&orchestrator);
}
