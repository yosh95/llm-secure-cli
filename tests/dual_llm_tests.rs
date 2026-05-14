use async_trait::async_trait;
use llm_secure_cli::cli::ui::{ConfirmResult, UserInterface};
use llm_secure_cli::core::context::AppContext;
use llm_secure_cli::llm::base::LlmClient;
use llm_secure_cli::llm::models::{ClientState, DataSource, LlmResponse};
use llm_secure_cli::security::dual_llm_verifier::{
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
/// This prevents tests from reading/writing to ~/.llm_secure_cli.
fn create_test_context() -> Arc<AppContext> {
    let tmp_dir = tempdir().expect("Failed to create temp dir");
    // Set HOME to tmp_dir for this process (Note: process-wide, but helps in isolation)
    unsafe {
        std::env::set_var("HOME", tmp_dir.path());
    }

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
async fn test_dual_llm_allow_scenario() {
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
async fn test_dual_llm_block_scenario() {
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
    if let VerificationOutcome::Rejected(reason) = outcome {
        assert!(reason.contains("Malicious delete"));
    } else {
        panic!("Expected Rejected, got {:?}", outcome);
    }
}

#[tokio::test]
async fn test_dual_llm_malformed_response() {
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
    if let VerificationOutcome::Rejected(reason) = outcome {
        assert!(reason.contains("Invalid verifier response format"));
    } else {
        panic!(
            "Expected Rejected due to malformed response, got {:?}",
            outcome
        );
    }
}

#[tokio::test]
async fn test_dual_llm_api_error_fallback() {
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
async fn test_dual_llm_tricky_response() {
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
        tool_name: "shell",
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
    if let VerificationOutcome::Rejected(reason) = outcome {
        assert!(
            reason.contains("Invalid verifier response format") || reason.contains("Security hole")
        );
    } else {
        panic!("Expected Rejected for tricky response, got {:?}", outcome);
    }
}
