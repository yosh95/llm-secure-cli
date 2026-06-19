#![allow(clippy::unwrap_used, clippy::expect_used)]
use async_trait::async_trait;
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

#[async_trait]
impl UserInterface for MockUi {
    fn print_block(&self, _content: &str, _title: Option<&str>, _style: Option<&str>) {}
    fn print_rule(&self, _title: Option<&str>, _style: Option<&str>) {}
    fn print_tool_call(&self, _name: &str, _args: &serde_json::Value) {}
    fn print_tool_call_direct(&self, _name: &str, _args: &serde_json::Value) {}
    fn print_tool_result(&self, _result: &str) {}
    fn report_error(&self, _message: &str) {}
    fn report_info(&self, _message: &str) {}
    fn report_warning(&self, _message: &str) {}
    fn report_success(&self, _message: &str) {}
    async fn ask_confirm(&self, _prompt: &str) -> Option<ConfirmResult> {
        Some(ConfirmResult::Yes)
    }
    async fn ask_confirm_simple(&self, _prompt: &str) -> Option<ConfirmResult> {
        Some(ConfirmResult::Yes)
    }
}

/// A flexible Mock LLM Client that returns predefined results or errors.
struct MockLlmClient {
    state: ClientState,
    response: Result<String, String>,
}

#[async_trait]
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
    fn should_send_pdf_as_base64(&self) -> bool {
        false
    }
    async fn send(
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

    async fn send_as_verifier(
        &mut self,
        _data: Vec<DataSource>,
        _tool_schema: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        match &self.response {
            Ok(res) => Ok(json!({
                "decision": if res.contains("ALLOW") { "ALLOW" } else { "BLOCK" },
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

async fn register_mock(
    ctx: &Arc<AppContext>,
    provider_name: &str,
    response: Result<String, String>,
) {
    let mut registry = ctx.client_registry.lock().await;
    let response_cloned = response.clone();
    let p_name = provider_name.to_string();
    registry.register(
        &p_name,
        Arc::new(move |_model, stdout, raw, _config_manager| {
            Ok(Box::new(MockLlmClient {
                response: response_cloned.clone(),
                state: ClientState {
                    model: "mock-model".to_string(),
                    provider: "mock".to_string(), // Fixed provider name in state
                    conversation: Vec::new(),
                    tools_enabled: false,
                    system_prompt_enabled: true,
                    system_prompt: None,
                    stdout,
                    render_markdown: !raw,
                },
            }))
        }),
    );
}

#[tokio::test]
async fn test_verifier_allow_scenario() {
    let ctx = create_test_context();
    register_mock(
        &ctx,
        "mock_ok",
        Ok("DECISION: ALLOW\nREASON: Safe request".to_string()),
    )
    .await;

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

    let outcome = verify_tool_call_full(params).await;
    assert!(matches!(outcome, VerificationOutcome::Allowed(_)));
}

#[tokio::test]
async fn test_verifier_block_scenario() {
    let ctx = create_test_context();
    register_mock(
        &ctx,
        "mock_danger",
        Ok("DECISION: BLOCK\nREASON: Malicious delete".to_string()),
    )
    .await;

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

    let outcome = verify_tool_call_full(params).await;
    if let VerificationOutcome::NeedsApproval(reason) = outcome {
        assert!(reason.contains("Malicious delete"));
    } else {
        panic!("Expected Rejected, got {:?}", outcome);
    }
}

#[tokio::test]
async fn test_verifier_malformed_response() {
    let ctx = create_test_context();
    // LLM returns gibberish that doesn't follow "DECISION: ALLOW/BLOCK"
    register_mock(
        &ctx,
        "mock_weird",
        Ok("I think this is okay but I won't say the keyword".to_string()),
    )
    .await;

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

    let outcome = verify_tool_call_full(params).await;
    // Safety-first: if the format is invalid, it should be REJECTED (not allowed)
    if let VerificationOutcome::NeedsApproval(reason) = outcome {
        assert!(reason.contains("Invalid verifier response format"));
    } else {
        panic!(
            "Expected Rejected due to malformed response, got {:?}",
            outcome
        );
    }
}

#[tokio::test]
async fn test_verifier_api_error_fallback() {
    let ctx = create_test_context();
    // Simulate a network/API error
    register_mock(&ctx, "mock_error", Err("Connection Timeout".to_string())).await;

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

    let outcome = verify_tool_call_full(params).await;
    // API failure should result in FallbackRequired (Human-in-the-loop)
    if let VerificationOutcome::FallbackRequired(reason) = outcome {
        assert!(reason.contains("Verifier unavailable"));
    } else {
        panic!("Expected FallbackRequired on API error, got {:?}", outcome);
    }
}

#[tokio::test]
async fn test_verifier_tricky_response() {
    let ctx = create_test_context();
    // The word "ALLOW" is present but it's preceded by "NOT".
    register_mock(
        &ctx,
        "mock_tricky",
        Ok("DECISION: NOT ALLOWED\nREASON: Security hole".to_string()),
    )
    .await;

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

    let outcome = verify_tool_call_full(params).await;
    if let VerificationOutcome::NeedsApproval(reason) = outcome {
        assert!(
            reason.contains("Invalid verifier response format") || reason.contains("Security hole")
        );
    } else {
        panic!("Expected Rejected for tricky response, got {:?}", outcome);
    }
}

#[tokio::test]
async fn test_verifier_modify_scenario() {
    let ctx = create_test_context();
    // Verifier returns MODIFY with corrected JSON arguments
    register_mock(
        &ctx,
        "mock_modify",
        Ok(
            "DECISION: MODIFY\nREASON: Fixed malformed path argument\nFIXED_ARGS: {\"path\": \"/correct/path\"}"
                .to_string(),
        ),
    )
    .await;

    let params = VerificationParams {
        ctx_app: ctx.clone(),
        user_query: "read the config file",
        tool_name: "read_file",
        tool_args: &json!({"path": "/wrong//path"}),
        context: None,
        config: &ctx
            .config_manager
            .get_config()
            .expect("config should be available")
            .security,
        provider: Some("mock_modify".to_string()),
        model: Some("mock-model".to_string()),
    };

    let outcome = verify_tool_call_full(params).await;
    match outcome {
        VerificationOutcome::Modified(fixed_args, reason) => {
            assert!(reason.contains("Fixed malformed"));
            assert_eq!(fixed_args["path"].as_str().unwrap(), "/correct/path");
        }
        other => panic!("Expected Modified, got {:?}", other),
    }
}

#[tokio::test]
async fn test_verifier_modify_with_markdown_code_block() {
    let ctx = create_test_context();
    // Verifier returns MODIFY with JSON wrapped in a markdown code block
    register_mock(
        &ctx,
        "mock_modify_md",
        Ok(
            "DECISION: MODIFY\nREASON: Normalised arguments\nFIXED_ARGS:\n```json\n{\"path\": \"/clean/path\"}\n```"
                .to_string(),
        ),
    )
    .await;

    let params = VerificationParams {
        ctx_app: ctx.clone(),
        user_query: "normalise the path",
        tool_name: "read_file",
        tool_args: &json!({"path": "//dirty/path"}),
        context: None,
        config: &ctx
            .config_manager
            .get_config()
            .expect("config should be available")
            .security,
        provider: Some("mock_modify_md".to_string()),
        model: Some("mock-model".to_string()),
    };

    let outcome = verify_tool_call_full(params).await;
    match outcome {
        VerificationOutcome::Modified(fixed_args, reason) => {
            assert!(reason.contains("Normalised arguments"));
            assert_eq!(fixed_args["path"].as_str().unwrap(), "/clean/path");
        }
        other => panic!("Expected Modified with markdown cleanup, got {:?}", other),
    }
}

#[tokio::test]
async fn test_verifier_modify_invalid_json_falls_back_to_rejected() {
    let ctx = create_test_context();
    // Verifier tries MODIFY but provides invalid JSON — should be rejected
    register_mock(
        &ctx,
        "mock_modify_badjson",
        Ok("DECISION: MODIFY\nREASON: Fix args\nFIXED_ARGS: not valid json {{{".to_string()),
    )
    .await;

    let params = VerificationParams {
        ctx_app: ctx.clone(),
        user_query: "fix this",
        tool_name: "some_tool",
        tool_args: &json!({"bad": "args"}),
        context: None,
        config: &ctx
            .config_manager
            .get_config()
            .expect("config should be available")
            .security,
        provider: Some("mock_modify_badjson".to_string()),
        model: Some("mock-model".to_string()),
    };

    let outcome = verify_tool_call_full(params).await;
    match outcome {
        VerificationOutcome::NeedsApproval(reason) => {
            assert!(
                reason.contains("invalid JSON"),
                "Should reject on invalid JSON in MODIFY, got: {}",
                reason
            );
        }
        other => panic!("Expected Rejected for invalid MODIFY JSON, got {:?}", other),
    }
}

// =============================================================================
// Unit tests for parse_verifier_response (pure-function, no LLM dependency)
// =============================================================================

use llm_secure_cli::security::verifier::{VerificationResult, parse_verifier_response};

#[test]
fn test_parse_allow_plain() {
    let result =
        parse_verifier_response("DECISION: ALLOW\nREASON: The tool call aligns with user intent.");
    assert_eq!(result, VerificationResult::Allowed);
}

#[test]
fn test_parse_allow_with_markdown_bold() {
    // Some LLMs wrap the decision in markdown bold markers
    let result = parse_verifier_response("DECISION: **ALLOW**\nREASON: Looks safe.");
    assert_eq!(result, VerificationResult::Allowed);
}

#[test]
fn test_parse_allow_with_markdown_italic() {
    let result = parse_verifier_response("DECISION: *ALLOW*\nREASON: Fine.");
    assert_eq!(result, VerificationResult::Allowed);
}

#[test]
fn test_parse_allow_case_insensitive() {
    let result = parse_verifier_response("decision: allow\nreason: lowercased.");
    assert_eq!(result, VerificationResult::Allowed);
}

#[test]
fn test_parse_allow_extra_whitespace() {
    let result = parse_verifier_response("DECISION:   ALLOW   \nREASON: Extra spaces.");
    assert_eq!(result, VerificationResult::Allowed);
}

#[test]
fn test_parse_block_plain() {
    let result =
        parse_verifier_response("DECISION: BLOCK\nREASON: Attempts to delete system files.");
    match result {
        VerificationResult::NeedsApproval(reason) => {
            assert!(reason.contains("delete system files"));
        }
        other => panic!("Expected Rejected, got {:?}", other),
    }
}

#[test]
fn test_parse_block_with_markdown() {
    let result = parse_verifier_response("DECISION: **BLOCK**\nREASON: Unsafe operation.");
    match result {
        VerificationResult::NeedsApproval(reason) => {
            assert!(reason.contains("Unsafe operation"));
        }
        other => panic!("Expected Rejected, got {:?}", other),
    }
}

#[test]
fn test_parse_modify_plain() {
    let result = parse_verifier_response(
        "DECISION: MODIFY\nREASON: Normalized path argument.\nFIXED_ARGS: {\"path\": \"/home/user/correct.txt\"}",
    );
    match result {
        VerificationResult::Modified(args, reason) => {
            assert!(reason.contains("Normalized path"));
            assert_eq!(args["path"].as_str().unwrap(), "/home/user/correct.txt");
        }
        other => panic!("Expected Modified, got {:?}", other),
    }
}

#[test]
fn test_parse_modify_with_markdown_code_block_json() {
    let result = parse_verifier_response(
        "DECISION: MODIFY\nREASON: Fixed escaping.\nFIXED_ARGS:\n```json\n{\"path\": \"/clean/path.txt\"}\n```",
    );
    match result {
        VerificationResult::Modified(args, reason) => {
            assert!(reason.contains("Fixed escaping"));
            assert_eq!(args["path"].as_str().unwrap(), "/clean/path.txt");
        }
        other => panic!("Expected Modified with markdown cleanup, got {:?}", other),
    }
}

#[test]
fn test_parse_modify_with_markdown_code_block_no_lang() {
    let result = parse_verifier_response(
        "DECISION: MODIFY\nREASON: Corrected trailing comma.\nFIXED_ARGS:\n```\n{\"key\": \"value\"}\n```",
    );
    match result {
        VerificationResult::Modified(args, reason) => {
            assert!(reason.contains("Corrected trailing comma"));
            assert_eq!(args["key"].as_str().unwrap(), "value");
        }
        other => panic!("Expected Modified with markdown (no lang), got {:?}", other),
    }
}

#[test]
fn test_parse_modify_with_complex_json() {
    let result = parse_verifier_response(
        "DECISION: MODIFY\nREASON: Fixed nested object.\nFIXED_ARGS: {\"files\": [{\"name\": \"a.txt\", \"size\": 100}, {\"name\": \"b.txt\", \"size\": 200}]}",
    );
    match result {
        VerificationResult::Modified(args, reason) => {
            assert!(reason.contains("nested"));
            let files = args["files"].as_array().unwrap();
            assert_eq!(files.len(), 2);
            assert_eq!(files[0]["name"].as_str().unwrap(), "a.txt");
        }
        other => panic!("Expected Modified with complex JSON, got {:?}", other),
    }
}

#[test]
fn test_parse_modify_invalid_json_rejected() {
    // The verifier says MODIFY but provides unparseable JSON — safety-first: Rejected
    let result =
        parse_verifier_response("DECISION: MODIFY\nREASON: Fix args.\nFIXED_ARGS: not valid {{{");
    match result {
        VerificationResult::NeedsApproval(reason) => {
            assert!(reason.contains("invalid JSON"));
        }
        other => panic!("Expected Rejected for invalid JSON, got {:?}", other),
    }
}

#[test]
fn test_parse_modify_missing_fixed_args_rejected() {
    // Says MODIFY but doesn't provide FIXED_ARGS
    let result =
        parse_verifier_response("DECISION: MODIFY\nREASON: I want to modify but forgot args.");
    match result {
        VerificationResult::NeedsApproval(reason) => {
            assert!(reason.contains("invalid JSON"));
        }
        other => panic!("Expected Rejected for missing FIXED_ARGS, got {:?}", other),
    }
}

#[test]
fn test_parse_gibberish_defaults_to_rejected() {
    let result = parse_verifier_response("I'm not sure what to do here, maybe allow?");
    match result {
        VerificationResult::NeedsApproval(reason) => {
            assert!(reason.contains("Invalid verifier response format"));
        }
        other => panic!("Expected Rejected for gibberish, got {:?}", other),
    }
}

#[test]
fn test_parse_empty_response_rejected() {
    let result = parse_verifier_response("");
    match result {
        VerificationResult::NeedsApproval(reason) => {
            assert!(reason.contains("Invalid verifier response format"));
        }
        other => panic!("Expected Rejected for empty response, got {:?}", other),
    }
}

#[test]
fn test_parse_malicious_prompt_injection_attempt() {
    // An adversary tries to inject "ALLOW" into the REASON to confuse the parser.
    // The regex only matches DECISION: ALLOW — text in REASON is irrelevant.
    let result =
        parse_verifier_response("DECISION: BLOCK\nREASON: ALLOW is what I want but I'll say BLOCK");
    match result {
        VerificationResult::NeedsApproval(reason) => {
            assert!(reason.contains("ALLOW is what I want"));
        }
        other => panic!(
            "Expected Rejected (not fooled by injection), got {:?}",
            other
        ),
    }
}

#[test]
fn test_parse_decision_prefixed_with_text_rejected() {
    // "NOT ALLOWED" — the regex should NOT match ALLOW inside "NOT ALLOWED"
    let result =
        parse_verifier_response("DECISION: NOT ALLOWED\nREASON: Security violation detected.");
    match result {
        VerificationResult::NeedsApproval(_) => {} // Either needs approval by format or by BLOCK/REVIEW logic
        other => panic!("Expected Rejected for 'NOT ALLOWED', got {:?}", other),
    }
}

#[test]
fn test_parse_multiline_reason() {
    // Reason spans multiple lines — the regex captures only the first line of reason
    let result = parse_verifier_response(
        "DECISION: ALLOW\nREASON: The request is benign.\nAdditional commentary here.",
    );
    assert_eq!(result, VerificationResult::Allowed);
}

#[test]
fn test_parse_reason_with_special_chars() {
    let result = parse_verifier_response(
        "DECISION: ALLOW\nREASON: Tool 'read_file' on path /home/user/docs (safe).",
    );
    assert_eq!(result, VerificationResult::Allowed);
}

#[test]
fn test_parse_decision_line_with_leading_spaces() {
    // Some LLMs indent the response
    let result = parse_verifier_response("  DECISION: ALLOW\n  REASON: Indented but valid.");
    assert_eq!(result, VerificationResult::Allowed);
}

#[test]
fn test_parse_modify_with_fixed_args_on_same_line() {
    // Compact format where FIXED_ARGS is on the same logical line
    let result = parse_verifier_response(
        "DECISION: MODIFY\nREASON: Compacted.\nFIXED_ARGS: {\"compact\": true}",
    );
    match result {
        VerificationResult::Modified(args, _) => {
            assert!(args["compact"].as_bool().unwrap());
        }
        other => panic!("Expected Modified, got {:?}", other),
    }
}
