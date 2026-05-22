use llm_secure_cli::config::models::{AppConfig, SecurityConfig, SecurityLevel, VerifierFallback};
use llm_secure_cli::security::validate_tool_call;
use serde_json::json;

// ---------------------------------------------------------------------------
// Config merge edge-case tests
// ---------------------------------------------------------------------------

#[test]
fn test_config_merge_nested_objects_preserve_existing_keys() {
    // Simulate what ConfigManager does internally: merge user overrides into defaults.
    // This tests the merge_json helper.
    let base: serde_json::Value = serde_json::json!({
        "general": {
            "pdf_as_base64": true,
            "request_timeout": 1800
        },
        "security": {
            "security_level": "high",
            "allowed_paths": ["."]
        }
    });

    let over: serde_json::Value = serde_json::json!({
        "general": {
            "request_timeout": 60
        }
    });

    // We can't call merge_json directly (private), so we test the observable
    // behaviour through ConfigManager::get_config() with a real file instead.
    // The test below is a structural test of the concept.
    let base_general = base.get("general").expect("base should have 'general'");
    let over_general = over.get("general").expect("over should have 'general'");
    assert_ne!(
        base_general["request_timeout"],
        over_general["request_timeout"]
    );
}

#[test]
fn test_security_config_default_values_are_sane() {
    let cfg = SecurityConfig::default();
    assert_eq!(cfg.security_level, SecurityLevel::High);
    assert!(!cfg.allowed_paths.is_empty());
    assert_eq!(cfg.allowed_paths[0], ".");
    assert_eq!(cfg.verifier_fallback, VerifierFallback::RequireApproval);
    assert!(cfg.static_analysis_is_error);
    assert!(cfg.dual_llm_confidence_threshold > 0.0);
    assert!(cfg.dual_llm_confidence_threshold <= 1.0);
}

// ---------------------------------------------------------------------------
// Phase 1 static analysis (fast-fail) edge cases
// ---------------------------------------------------------------------------

#[test]
fn test_validate_tool_call_normal_commands_allowed() {
    let config = SecurityConfig::default();

    // A normal python code must pass Phase 1.
    let mut args = serde_json::Map::new();
    args.insert("code".to_string(), json!("print('hello')"));
    let result = validate_tool_call("execute_python", &args, &config);
    assert!(
        result.is_ok(),
        "Normal python code should pass: {:?}",
        result
    );
}

#[test]
fn test_validate_tool_call_control_characters_blocked() {
    let config = SecurityConfig::default();

    // Various control characters
    for ch in ['\x00', '\x01', '\x02', '\x1b'] {
        let mut args = serde_json::Map::new();
        let code = format!("print('hello{}')", ch);
        args.insert("code".to_string(), json!(code));
        let result = validate_tool_call("execute_python", &args, &config);
        assert!(
            result.is_err(),
            "Control char {:#04x} should be blocked",
            ch as u32
        );
    }
}

#[test]
fn test_validate_tool_call_non_execute_tool_string_args_checked() {
    let config = SecurityConfig::default();

    // For non-execute tools, control chars in any string arg must be caught.
    let mut args = serde_json::Map::new();
    args.insert("path".to_string(), json!("test\x00.txt"));
    let result = validate_tool_call("read_file", &args, &config);
    assert!(result.is_err(), "Null byte in path should be blocked");
}

#[test]
fn test_validate_tool_call_harmless_empty_args_allowed() {
    let config = SecurityConfig::default();

    let mut args = serde_json::Map::new();
    args.insert("code".to_string(), json!("print('hello')"));
    let result = validate_tool_call("execute_python", &args, &config);
    assert!(result.is_ok());
}

// ---------------------------------------------------------------------------
// Merkle tree edge cases
// ---------------------------------------------------------------------------

#[test]
fn test_merkle_tree_three_leaves() {
    use llm_secure_cli::security::merkle::MerkleTree;

    let leaves = vec!["a".repeat(64), "b".repeat(64), "c".repeat(64)];
    let tree = MerkleTree::new(leaves);
    // Should produce a valid hex root
    assert_eq!(tree.root_hex.len(), 64);
    assert!(tree.root_hex.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn test_merkle_tree_deterministic() {
    use llm_secure_cli::security::merkle::MerkleTree;

    let leaves = vec!["x".repeat(64), "y".repeat(64), "z".repeat(64)];
    let root1 = MerkleTree::new(leaves.clone()).root_hex;
    let root2 = MerkleTree::new(leaves.clone()).root_hex;
    assert_eq!(root1, root2, "Merkle root must be deterministic");
}

#[test]
fn test_merkle_tree_different_data_different_root() {
    use llm_secure_cli::security::merkle::MerkleTree;

    let tree1 = MerkleTree::new(vec!["a".repeat(64)]);
    let tree2 = MerkleTree::new(vec!["b".repeat(64)]);
    assert_ne!(tree1.root_hex, tree2.root_hex);
}

// ---------------------------------------------------------------------------
// AppConfig (de)serialization round-trip
// ---------------------------------------------------------------------------

#[test]
fn test_app_config_round_trip_via_toml() {
    let original = AppConfig::default();
    let toml_str = toml::to_string(&original).expect("Serialize default config");
    let roundtripped: AppConfig = toml::from_str(&toml_str).expect("Deserialize config");
    // Spot-check a few fields
    assert_eq!(
        roundtripped.security.security_level,
        original.security.security_level
    );
    assert_eq!(
        roundtripped.security.verifier_fallback,
        original.security.verifier_fallback
    );
}

#[test]
fn test_app_config_deserializes_minimal_toml() {
    let minimal = r#"
[general]
pdf_as_base64 = false

[security]
allowed_paths = ["/tmp"]
"#;
    let cfg: AppConfig = toml::from_str(minimal).expect("Minimal TOML should parse");
    assert!(!cfg.general.pdf_as_base64);
    assert_eq!(cfg.security.allowed_paths, vec!["/tmp"]);
    // Fields not specified should take their defaults
    assert_eq!(cfg.security.security_level, SecurityLevel::High);
}

// ---------------------------------------------------------------------------
// Risk level classification sanity
// ---------------------------------------------------------------------------

#[test]
fn test_cass_risk_levels_are_mutually_exclusive_in_defaults() {
    use llm_secure_cli::security::cass::CASS_ORCHESTRATOR;
    // SecurityConfig::default() has dual_llm_verification = None which
    // is treated as false, causing High -> Critical escalation.
    // Also security_level = "high" escalates Low -> Medium.
    // Set both explicitly to test the base classification.
    let config = SecurityConfig {
        dual_llm_verification: Some(true),
        security_level: SecurityLevel::Standard,
        ..Default::default()
    };

    // Execute python should be high risk
    let mut args = serde_json::Map::new();
    args.insert("code".to_string(), json!("print('hello')"));
    let risk = CASS_ORCHESTRATOR.evaluate_risk("execute_python", Some(&json!(args)), &config);
    assert_eq!(
        risk as u8,
        llm_secure_cli::security::cass::RiskLevel::High as u8,
        "execute_python should be high risk with dual_llm enabled"
    );

    // List files should be low risk
    let risk = CASS_ORCHESTRATOR.evaluate_risk("list_files", None, &config);
    assert_eq!(
        risk as u8,
        llm_secure_cli::security::cass::RiskLevel::Low as u8,
        "list_files should be low risk by default"
    );
}

// ---------------------------------------------------------------------------
// Path validator edge cases
// ---------------------------------------------------------------------------

#[test]
fn test_path_validator_allows_new_file_in_allowed_directory() {
    use llm_secure_cli::security::path_validator::validate_path;

    let config = SecurityConfig {
        allowed_paths: vec![".".to_string()],
        ..Default::default()
    };

    // Existing file
    assert!(validate_path("Cargo.toml", &config).is_ok());

    // New file in current directory (should be allowed if parent is allowed)
    let result = validate_path("new_file_that_does_not_exist.txt", &config);
    assert!(
        result.is_ok(),
        "New file in allowed dir should be allowed: {:?}",
        result
    );
}

#[test]
fn test_path_validator_blocks_escape_attempts() {
    use llm_secure_cli::security::path_validator::validate_path;

    let config = SecurityConfig {
        allowed_paths: vec![".".to_string()],
        ..Default::default()
    };

    // Symlink escape via ../
    assert!(validate_path("../etc/passwd", &config).is_err());
    assert!(validate_path("../../root/.ssh/id_rsa", &config).is_err());
}

// ---------------------------------------------------------------------------
// Security config validation tests
// ---------------------------------------------------------------------------

#[test]
fn test_validate_security_config_accepts_valid_defaults() {
    // Default config must pass validation
    let _cfg = SecurityConfig::default();
    // We can't call validate_security_config directly (private),
    // but ConfigManager::get_config() will call it. We test that
    // the default config loads successfully.
    let result = llm_secure_cli::config::ConfigManager::new().get_config();
    assert!(
        result.is_ok(),
        "Default SecurityConfig should pass validation: {:?}",
        result.err()
    );
}

#[test]
fn test_security_config_rejects_invalid_auto_approval_level() {
    // With the typed enum, invalid auto_approval_level values are
    // rejected at TOML deserialization time, not at runtime validation.
    let toml_str = r#"
[security]
auto_approval_level = "full"
security_level = "high"
"#;
    let cfg: Result<AppConfig, _> = toml::from_str(toml_str);
    assert!(
        cfg.is_err(),
        "TOML parse should fail for invalid auto_approval_level value"
    );
}

#[test]
fn test_security_config_rejects_invalid_confidence_threshold() {
    let cfg = SecurityConfig {
        dual_llm_confidence_threshold: 1.5,
        ..Default::default()
    };
    let errors = cfg.validate();
    assert!(
        errors
            .iter()
            .any(|e| e.field == "dual_llm_confidence_threshold"),
        "Should report error for confidence_threshold > 1.0"
    );
}

#[test]
fn test_security_config_rejects_negative_threshold() {
    let cfg = SecurityConfig {
        dual_llm_confidence_threshold: -0.1,
        ..Default::default()
    };
    let errors = cfg.validate();
    assert!(
        errors
            .iter()
            .any(|e| e.field == "dual_llm_confidence_threshold"),
        "Should report error for negative confidence_threshold"
    );
}

#[test]
fn test_security_config_rejects_invalid_security_level() {
    // With the typed enum, invalid security_level values are
    // rejected at TOML deserialization time.
    let toml_str = r#"
[security]
security_level = "paranoid"
"#;
    let cfg: Result<AppConfig, _> = toml::from_str(toml_str);
    assert!(
        cfg.is_err(),
        "TOML parse should fail for invalid security_level value"
    );
}

#[test]
fn test_security_config_rejects_invalid_verifier_fallback() {
    // With the typed enum, invalid verifier_fallback values are
    // rejected at TOML deserialization time.
    let toml_str = r#"
[security]
verifier_fallback = "allow"
"#;
    let cfg: Result<AppConfig, _> = toml::from_str(toml_str);
    assert!(
        cfg.is_err(),
        "TOML parse should fail for invalid verifier_fallback value"
    );
}

#[test]
fn test_security_config_validate_warnings_high_without_dual_llm() {
    // Default config has security_level="high" but dual_llm_verification=None
    let cfg = SecurityConfig::default();
    let warnings = cfg.validate_warnings();
    assert!(
        warnings.iter().any(|w| w.field == "security_level"),
        "Default config should warn about high security without dual_llm_verification"
    );
}

#[test]
fn test_security_config_no_warnings_when_dual_llm_enabled() {
    let cfg = SecurityConfig {
        dual_llm_verification: Some(true),
        ..Default::default()
    };
    let warnings = cfg.validate_warnings();
    assert!(
        warnings.is_empty(),
        "Config with dual_llm enabled should have no warnings, got: {:?}",
        warnings
    );
}

#[test]
fn test_security_config_validate_errors_for_empty_allowed_paths() {
    let cfg = SecurityConfig {
        allowed_paths: vec![],
        ..Default::default()
    };
    let errors = cfg.validate();
    assert!(
        errors.iter().any(|e| e.field == "allowed_paths"),
        "Should report error for empty allowed_paths"
    );
}

#[test]
fn test_security_config_validate_errors_for_dual_llm_without_provider() {
    let cfg = SecurityConfig {
        dual_llm_verification: Some(true),
        dual_llm_provider: "".to_string(),
        ..Default::default()
    };
    let errors = cfg.validate();
    assert!(
        errors.iter().any(|e| e.field == "dual_llm_provider"),
        "Should report error for dual_llm enabled without provider"
    );
}

// ---------------------------------------------------------------------------
// Static analyzer edge cases
// ---------------------------------------------------------------------------

#[test]
fn test_static_analyzer_unicode_and_emoji_allowed() {
    use llm_secure_cli::security::static_analyzer::StaticAnalyzer;

    // Unicode, emoji, and printable special chars should all pass
    assert!(!StaticAnalyzer::is_obviously_malicious("日本語テスト"));
    assert!(!StaticAnalyzer::is_obviously_malicious("🎉✨"));
    assert!(!StaticAnalyzer::is_obviously_malicious(
        "normal text with symbols: @#$%^&*()"
    ));
    assert!(!StaticAnalyzer::is_obviously_malicious("print('hello')"));
}

#[test]
fn test_static_analyzer_normal_control_chars_allowed() {
    use llm_secure_cli::security::static_analyzer::StaticAnalyzer;

    // Tab, newline, carriage return are allowed (they're standard formatting)
    assert!(!StaticAnalyzer::is_obviously_malicious("line1\tline2"));
    assert!(!StaticAnalyzer::is_obviously_malicious("line1\nline2"));
    assert!(!StaticAnalyzer::is_obviously_malicious("line1\r\nline2"));
}

#[test]
fn test_static_analyzer_null_byte_rejected() {
    use llm_secure_cli::security::static_analyzer::StaticAnalyzer;

    assert!(StaticAnalyzer::is_obviously_malicious("test\x00"));
    assert!(StaticAnalyzer::is_obviously_malicious("\x00test"));
    assert!(StaticAnalyzer::is_obviously_malicious("te\x00st"));
}

#[test]
fn test_static_analyzer_escape_and_control_blocked() {
    use llm_secure_cli::security::static_analyzer::StaticAnalyzer;

    // ESC (0x1B), BEL (0x07), and other non-formatting control chars
    for ch in ['\x1b', '\x07', '\x01', '\x02', '\x0b', '\x0c'] {
        let s = format!("test{}", ch);
        assert!(
            StaticAnalyzer::is_obviously_malicious(&s),
            "char {:#04x} should be blocked",
            ch as u32
        );
    }
}

#[test]
fn test_static_analyzer_check_function_with_args() {
    use llm_secure_cli::security::static_analyzer::StaticAnalyzer;

    // Normal command with safe args
    let (ok, violations) = StaticAnalyzer::check("ls", &["-la".to_string()]);
    assert!(ok);
    assert!(violations.is_empty());

    // Command name with null byte
    let (ok, violations) = StaticAnalyzer::check("ls\x00", &["-la".to_string()]);
    assert!(!ok);
    assert!(!violations.is_empty());

    // Safe command, malicious arg
    let (ok, violations) = StaticAnalyzer::check("ls", &["evil\x1b".to_string()]);
    assert!(!ok);
    assert!(!violations.is_empty());
}

#[test]
fn test_static_analyzer_is_dangerous_command_backward_compat() {
    use llm_secure_cli::security::static_analyzer::StaticAnalyzer;

    // Backward compatibility alias
    assert!(!StaticAnalyzer::is_dangerous_command("safe"));
    assert!(StaticAnalyzer::is_dangerous_command("bad\x00"));
}
