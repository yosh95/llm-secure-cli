use llm_secure_cli::config::models::SecurityConfig;
use llm_secure_cli::security::cass::{CASS_ORCHESTRATOR, RiskLevel};

#[test]
fn test_cass_risk_evaluation() {
    let mut config = SecurityConfig {
        dual_llm_verification: Some(false),
        ..SecurityConfig::default()
    };

    // 1. Test critical risk (execute_command without dual LLM)
    let risk = CASS_ORCHESTRATOR.evaluate_risk("execute_command", None, &config);
    assert!(matches!(risk, RiskLevel::Critical));

    // 2. Test high risk (execute_command with dual LLM)
    config.dual_llm_verification = Some(true);
    let risk = CASS_ORCHESTRATOR.evaluate_risk("execute_command", None, &config);
    assert!(matches!(risk, RiskLevel::High));

    // 3. Test explicit high risk tool
    config.high_risk_tools = vec!["create_or_overwrite_file".to_string()];
    let risk = CASS_ORCHESTRATOR.evaluate_risk("create_or_overwrite_file", None, &config);
    assert!(matches!(risk, RiskLevel::High));
}

#[test]
fn test_cass_security_posture_requirements() {
    let config = SecurityConfig {
        dual_llm_verification: Some(true),
        ..SecurityConfig::default()
    };

    // High risk requirements
    let posture = CASS_ORCHESTRATOR.get_security_requirements("execute_command", None, &config);
    assert!(posture.require_pqc_signature);
    assert_eq!(posture.pqc_variant, "ML-DSA-87");
    assert!(posture.require_pqc_audit_encryption);
    assert_eq!(posture.ast_strictness, "strict");
    assert!(posture.require_dual_llm_verification);

    // Low risk requirements
    let posture = CASS_ORCHESTRATOR.get_security_requirements("list_files", None, &config);
    assert!(posture.require_pqc_signature);
    assert_eq!(posture.pqc_variant, "ML-DSA-44");
}
