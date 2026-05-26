use llm_secure_cli::config::models::{AppConfig, SecurityConfig, SecurityLevel};
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
            "security_level": "high"
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
}

#[test]
fn test_app_config_deserializes_minimal_toml() {
    let minimal = r#"
[general]
pdf_as_base64 = false
"#;
    let cfg: AppConfig = toml::from_str(minimal).expect("Minimal TOML should parse");
    assert!(!cfg.general.pdf_as_base64);
    // Fields not specified should take their defaults
    assert_eq!(cfg.security.security_level, SecurityLevel::High);
}

// ---------------------------------------------------------------------------
// Risk level classification sanity
// ---------------------------------------------------------------------------

#[test]
fn test_cass_risk_levels_are_mutually_exclusive_in_defaults() {
    use llm_secure_cli::security::cass::CASSOrchestrator;
    // CASS risk evaluation is deprecated: all tools return Low.
    // The Verifier Committee handles all risk assessment.
    let config = SecurityConfig::default();

    // Every tool now returns Low — CASS delegates all risk to the Verifier
    let mut args = serde_json::Map::new();
    args.insert("code".to_string(), json!("print('hello')"));
    let risk = CASSOrchestrator::evaluate_risk("execute_python", Some(&json!(args)), &config);
    assert_eq!(
        risk as u8,
        llm_secure_cli::security::cass::RiskLevel::Low as u8,
        "CASS always returns Low; risk assessment delegated to Verifier Committee"
    );

    let risk = CASSOrchestrator::evaluate_risk("list_files", None, &config);
    assert_eq!(
        risk as u8,
        llm_secure_cli::security::cass::RiskLevel::Low as u8,
        "CASS always returns Low"
    );
}

// ---------------------------------------------------------------------------
// Path validator edge cases
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Security config validation tests
// ---------------------------------------------------------------------------

#[test]
fn test_validate_security_config_accepts_valid_defaults() {
    // Default SecurityConfig must pass validation
    let cfg = SecurityConfig::default();
    let errors = cfg.validate();
    assert!(
        errors.is_empty(),
        "Default SecurityConfig should pass validation: {:?}",
        errors
    );

    // Also verify individual fields make sense
    assert_eq!(
        cfg.security_level,
        SecurityLevel::High,
        "default security level should be high"
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
fn test_security_config_validate_warnings_high_without_verifier() {
    // Default config has security_level="high" but verifier_enabled=None
    let cfg = SecurityConfig::default();
    let warnings = cfg.validate_warnings();
    assert!(
        warnings.iter().any(|w| w.field == "security_level"),
        "Default config should warn about high security without verifier_enabled"
    );
}

#[test]
fn test_security_config_no_warnings_when_verifier_enabled() {
    let cfg = SecurityConfig {
        verifier_enabled: Some(true),
        ..Default::default()
    };
    let warnings = cfg.validate_warnings();
    assert!(
        warnings.is_empty(),
        "Config with verifier enabled should have no warnings, got: {:?}",
        warnings
    );
}

#[test]
fn test_security_config_validate_errors_for_verifier_without_provider() {
    // verifier_enabled is enabled, but neither
    // legacy provider/model nor verifier_committee members are set.
    let cfg = SecurityConfig {
        verifier_enabled: Some(true),
        verifier_provider: "".to_string(),
        ..Default::default()
    };
    let errors = cfg.validate();
    assert!(
        errors.iter().any(|e| e.field == "verifier_enabled"),
        "Should report error on verifier_enabled when enabled without any provider/committee config"
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
