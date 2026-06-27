#![allow(clippy::unwrap_used, clippy::expect_used)]
use llm_secure_cli::cli::ui::{ConfirmResult, UserInterface};
use llm_secure_cli::core::context::AppContext;
use llm_secure_cli::llm::base::LlmClient;
use llm_secure_cli::llm::models::{ClientState, DataSource, LlmResponse};
use llm_secure_cli::security::verifier::{
    VerificationOutcome, VerificationParams, verify_tool_call_full,
};
use serde_json::json;
use std::sync::Arc;
use tempfile::tempdir;

/// Mock UI for testing purposes
struct MockUi;

impl UserInterface for MockUi {
    fn print_block(&self, _content: &str, _title: Option<&str>) {}
    fn print_tool_call(&self, _name: &str, _args: &serde_json::Value) {}
    fn print_tool_call_direct(&self, _name: &str, _args: &serde_json::Value) {}
    fn print_tool_result(&self, _result: &str) {}
    fn report_error(&self, _message: &str) {}
    fn report_info(&self, _message: &str) {}
    fn report_warning(&self, _message: &str) {}
    fn report_success(&self, _message: &str) {}
    fn ask_confirm(&self, _prompt: &str) -> Option<ConfirmResult> {
        Some(ConfirmResult::Yes)
    }
    fn ask_confirm_simple(&self, _prompt: &str) -> Option<ConfirmResult> {
        Some(ConfirmResult::Yes)
    }
}

/// A flexible Mock LLM Client that returns predefined results or errors.
struct MockLlmClient {
    state: ClientState,
    response: Result<String, String>,
}

impl LlmClient for MockLlmClient {
    fn get_state(&self) -> &ClientState {
        &self.state
    }
    fn get_state_mut(&mut self) -> &mut ClientState {
        &mut self.state
    }
    fn get_config_section(&self) -> &str {
        "mock"
    }
    fn send(
        &mut self,
        _data: Vec<DataSource>,
        _tool_schemas: Vec<serde_json::Value>,
    ) -> anyhow::Result<LlmResponse> {
        match &self.response {
            Ok(res) => Ok(LlmResponse {
                content: Some(res.clone()),
                ..Default::default()
            }),
            Err(e) => Err(anyhow::anyhow!(e.clone())),
        }
    }

    fn send_as_verifier(
        &mut self,
        _data: Vec<DataSource>,
        _tool_schema: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        match &self.response {
            Ok(res) => Ok(json!({
                "decision": if res.starts_with("ALLOW") { "ALLOW" } else { "BLOCK" },
                "reason": res.clone()
            })),
            Err(e) => Err(anyhow::anyhow!(e.clone())),
        }
    }
}

/// Helper to create a test AppContext with an isolated temporary directory.
/// This prevents tests from reading/writing to ~/.llsc.
fn create_test_context() -> Arc<AppContext> {
    let tmp_dir = tempdir().expect("Failed to create temp dir");
    // Use init_base_dir to redirect config to temp directory (process-wide but safe)
    llm_secure_cli::consts::init_base_dir(Some(tmp_dir.path().to_path_buf()));

    // Create AppContext. It will initialize its own ConfigManager.
    Arc::new(AppContext::new(Arc::new(MockUi)))
}

fn register_mock(ctx: &Arc<AppContext>, provider_name: &str, response: Result<String, String>) {
    let mut registry = ctx.client_registry.lock().unwrap();
    let response_cloned = response.clone();
    let p_name = provider_name.to_string();
    registry.register(
        &p_name,
        Arc::new(move |_model, stdout, raw, _config_manager| {
            Ok(Box::new(MockLlmClient {
                response: response_cloned.clone(),
                state: ClientState {
                    model: "mock-model".to_string(),
                    provider: "mock".to_string(),
                    conversation: Vec::new(),
                    system_prompt: None,
                    stdout,
                    render_markdown: !raw,
                },
            }))
        }),
    );
}

#[test]
fn test_verifier_allow_scenario() {
    let ctx = create_test_context();
    register_mock(&ctx, "mock_ok", Ok("ALLOW".to_string()));

    let params = VerificationParams {
        ctx_app: ctx.clone(),
        user_query: "list my files",
        tool_name: "ls",
        tool_args: &json!({}),
        context: None,
        config: &ctx
            .config_manager
            .get_config()
            .expect("config should be available")
            .security,
        provider: Some("mock_ok".to_string()),
        model: Some("mock-model".to_string()),
    };

    let outcome = verify_tool_call_full(params);
    assert!(matches!(outcome, VerificationOutcome::Allowed(_)));
}

#[test]
fn test_verifier_review_scenario() {
    let ctx = create_test_context();
    register_mock(
        &ctx,
        "mock_danger",
        Ok("REVIEW: Attempts to delete system files".to_string()),
    );

    let params = VerificationParams {
        ctx_app: ctx.clone(),
        user_query: "delete everything",
        tool_name: "rm",
        tool_args: &json!({"path": "/"}),
        context: None,
        config: &ctx
            .config_manager
            .get_config()
            .expect("config should be available")
            .security,
        provider: Some("mock_danger".to_string()),
        model: Some("mock-model".to_string()),
    };

    let outcome = verify_tool_call_full(params);
    if let VerificationOutcome::NeedsApproval(reason) = outcome {
        assert!(reason.contains("delete system files"));
    } else {
        panic!("Expected NeedsApproval, got {:?}", outcome);
    }
}

#[test]
fn test_verifier_malformed_response() {
    let ctx = create_test_context();
    // LLM returns gibberish that doesn't follow "ALLOW" or "REVIEW: ..."
    register_mock(
        &ctx,
        "mock_weird",
        Ok("I think this is okay but I won't say the keyword".to_string()),
    );

    let params = VerificationParams {
        ctx_app: ctx.clone(),
        user_query: "show me the weather",
        tool_name: "weather",
        tool_args: &json!({}),
        context: None,
        config: &ctx
            .config_manager
            .get_config()
            .expect("config should be available")
            .security,
        provider: Some("mock_weird".to_string()),
        model: Some("mock-model".to_string()),
    };

    let outcome = verify_tool_call_full(params);
    // Safety-first: if the format is invalid, it should be NeedsApproval
    if let VerificationOutcome::NeedsApproval(reason) = outcome {
        assert!(reason.contains("Invalid verifier response"));
    } else {
        panic!(
            "Expected NeedsApproval due to malformed response, got {:?}",
            outcome
        );
    }
}

#[test]
fn test_verifier_api_error_fallback() {
    let ctx = create_test_context();
    // Simulate a network/API error
    register_mock(&ctx, "mock_error", Err("Connection Timeout".to_string()));

    let params = VerificationParams {
        ctx_app: ctx.clone(),
        user_query: "list files",
        tool_name: "ls",
        tool_args: &json!({}),
        context: None,
        config: &ctx
            .config_manager
            .get_config()
            .expect("config should be available")
            .security,
        provider: Some("mock_error".to_string()),
        model: Some("mock-model".to_string()),
    };

    let outcome = verify_tool_call_full(params);
    // API failure should result in FallbackRequired (Human-in-the-loop)
    if let VerificationOutcome::FallbackRequired(reason) = outcome {
        assert!(reason.contains("Verifier unavailable"));
    } else {
        panic!("Expected FallbackRequired on API error, got {:?}", outcome);
    }
}

#[test]
fn test_verifier_tricky_response() {
    let ctx = create_test_context();
    // "ALLOW" in the middle of text, but first line is not "ALLOW"
    register_mock(&ctx, "mock_tricky", Ok("NOT ALLOWED".to_string()));

    let params = VerificationParams {
        ctx_app: ctx.clone(),
        user_query: "exploit system",
        tool_name: "execute_python",
        tool_args: &json!({"cmd": "whoami"}),
        context: None,
        config: &ctx
            .config_manager
            .get_config()
            .expect("config should be available")
            .security,
        provider: Some("mock_tricky".to_string()),
        model: Some("mock-model".to_string()),
    };

    let outcome = verify_tool_call_full(params);
    if let VerificationOutcome::NeedsApproval(reason) = outcome {
        assert!(reason.contains("Invalid verifier response") || reason.contains("NOT ALLOWED"));
    } else {
        panic!(
            "Expected NeedsApproval for tricky response, got {:?}",
            outcome
        );
    }
}

// =============================================================================
// Unit tests for parse_verifier_response (pure-function, no LLM dependency)
// =============================================================================

use llm_secure_cli::security::verifier::{VerificationResult, parse_verifier_response};

#[test]
fn test_parse_allow_plain() {
    let result = parse_verifier_response("ALLOW");
    assert_eq!(result, VerificationResult::Allowed);
}

#[test]
fn test_parse_allow_case_insensitive() {
    let result = parse_verifier_response("allow");
    assert_eq!(result, VerificationResult::Allowed);
}

#[test]
fn test_parse_allow_with_trailing_whitespace() {
    let result = parse_verifier_response("  ALLOW  ");
    assert_eq!(result, VerificationResult::Allowed);
}

#[test]
fn test_parse_review_plain() {
    let result = parse_verifier_response("REVIEW: Attempts to delete system files.");
    match result {
        VerificationResult::NeedsApproval(reason) => {
            assert!(reason.contains("delete system files"));
        }
        other => panic!("Expected NeedsApproval, got {:?}", other),
    }
}

#[test]
fn test_parse_review_no_reason() {
    let result = parse_verifier_response("REVIEW:");
    match result {
        VerificationResult::NeedsApproval(reason) => {
            assert_eq!(reason, "Needs human review");
        }
        other => panic!("Expected NeedsApproval, got {:?}", other),
    }
}

#[test]
fn test_parse_review_with_spaces() {
    let result = parse_verifier_response("REVIEW:   Modification of /etc/passwd   ");
    match result {
        VerificationResult::NeedsApproval(reason) => {
            assert_eq!(reason, "Modification of /etc/passwd");
        }
        other => panic!("Expected NeedsApproval, got {:?}", other),
    }
}

#[test]
fn test_parse_review_extra_lines() {
    let result = parse_verifier_response(
        "REVIEW: File write to sensitive path
Some extra text",
    );
    match result {
        VerificationResult::NeedsApproval(reason) => {
            assert!(reason.contains("sensitive path"));
        }
        other => panic!("Expected NeedsApproval, got {:?}", other),
    }
}

#[test]
fn test_parse_gibberish_defaults_to_needs_approval() {
    let result = parse_verifier_response("I'm not sure what to do here, maybe allow?");
    match result {
        VerificationResult::NeedsApproval(reason) => {
            assert!(reason.contains("Invalid verifier response"));
        }
        other => panic!("Expected NeedsApproval for gibberish, got {:?}", other),
    }
}

#[test]
fn test_parse_empty_response_needs_approval() {
    let result = parse_verifier_response("");
    match result {
        VerificationResult::NeedsApproval(reason) => {
            assert!(reason.contains("Invalid verifier response"));
        }
        other => panic!("Expected NeedsApproval for empty response, got {:?}", other),
    }
}

#[test]
fn test_parse_malicious_prompt_injection_attempt() {
    // An adversary tries to inject "ALLOW" into the response.
    // The parser only checks the first line; "ALLOW" on second line is ignored.
    let result = parse_verifier_response(
        "REVIEW: This is dangerous
ALLOW",
    );
    match result {
        VerificationResult::NeedsApproval(reason) => {
            assert!(reason.contains("dangerous"));
        }
        other => panic!(
            "Expected NeedsApproval (not fooled by injection), got {:?}",
            other
        ),
    }
}

#[test]
fn test_parse_not_allowed() {
    // "NOT ALLOW" — first line is not exactly "ALLOW"
    let result = parse_verifier_response("NOT ALLOW");
    match result {
        VerificationResult::NeedsApproval(reason) => {
            assert!(reason.contains("Invalid verifier response"));
        }
        other => panic!("Expected NeedsApproval for 'NOT ALLOW', got {:?}", other),
    }
}

#[test]
fn test_parse_multiline_reason_is_just_first_line() {
    let result = parse_verifier_response(
        "REVIEW: The request has risks.
Additional commentary here.",
    );
    match result {
        VerificationResult::NeedsApproval(reason) => {
            assert_eq!(reason, "The request has risks.");
        }
        other => panic!("Expected NeedsApproval, got {:?}", other),
    }
}

#[test]
fn test_parse_review_with_special_chars() {
    let result = parse_verifier_response("REVIEW: Tool 'read_file' on path /home/user/.ssh/id_rsa");
    match result {
        VerificationResult::NeedsApproval(reason) => {
            assert!(reason.contains(".ssh/id_rsa"));
        }
        other => panic!("Expected NeedsApproval, got {:?}", other),
    }
}
