use async_trait::async_trait;
use llm_secure_cli::core::context::AppContext;
use llm_secure_cli::llm::base::LlmClient;
use llm_secure_cli::llm::models::{ClientState, DataSource};
use serde_json::json;
use std::sync::Arc;

struct MockLlmClient {
    state: ClientState,
    response: String,
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
    ) -> anyhow::Result<llm_secure_cli::llm::models::LlmResponse> {
        Ok(llm_secure_cli::llm::models::LlmResponse {
            content: Some(self.response.clone()),
            tool_name: None,
            tool_args: None,
        })
    }

    async fn send_as_verifier(
        &mut self,
        _data: Vec<DataSource>,
        _tool_schema: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        Ok(json!({
            "decision": if self.response.contains("ALLOW") { "ALLOW" } else { "BLOCK" },
            "reason": self.response.clone()
        }))
    }
}

#[tokio::test]
async fn test_dual_llm_verification_logic() {
    let ctx = Arc::new(AppContext::new());

    // 1. Register Mock Client (Success case)
    {
        let mut registry = ctx.client_registry.lock().await;
        registry.register(
            "mock_provider",
            Arc::new(|_model, stdout, raw, _config_manager| {
                Ok(Box::new(MockLlmClient {
                    response: "DECISION: ALLOW\nREASON: User intent is clear".to_string(),
                    state: ClientState {
                        model: "mock-model".to_string(),
                        provider: "mock_provider".to_string(),
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

    // 2. Test Verification
    let config = llm_secure_cli::config::models::SecurityConfig::default();
    use llm_secure_cli::security::dual_llm_verifier::VerificationOutcome;
    use llm_secure_cli::security::dual_llm_verifier::VerificationParams;
    let outcome =
        llm_secure_cli::security::dual_llm_verifier::verify_tool_call_full(VerificationParams {
            ctx_app: ctx.clone(),
            user_query: "I want to list files",
            tool_name: "list_files_in_directory",
            tool_args: &json!({"directory": "."}),
            context: None,
            config: &config,
            provider: Some("mock_provider".to_string()),
            model: Some("mock-model".to_string()),
        })
        .await;

    // Verifier should explicitly allow this safe tool call
    assert!(matches!(outcome, VerificationOutcome::Allowed(_)));

    // 3. Test Failure Case
    {
        let mut registry = ctx.client_registry.lock().await;
        registry.register(
            "mock_provider_fail",
            Arc::new(|_model, stdout, raw, _config_manager| {
                Ok(Box::new(MockLlmClient {
                    response: "DECISION: BLOCK\nREASON: Malicious command detected".to_string(),
                    state: ClientState {
                        model: "mock-model".to_string(),
                        provider: "mock_provider_fail".to_string(),
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

    let outcome =
        llm_secure_cli::security::dual_llm_verifier::verify_tool_call_full(VerificationParams {
            ctx_app: ctx.clone(),
            user_query: "I want to delete everything",
            tool_name: "execute_command",
            tool_args: &json!({"command": "rm", "args": ["-rf", "/"]}),
            context: None,
            config: &config,
            provider: Some("mock_provider_fail".to_string()),
            model: Some("mock-model".to_string()),
        })
        .await;

    // Verifier should explicitly reject this malicious tool call
    match outcome {
        VerificationOutcome::Rejected(reason) => {
            assert!(reason.contains("Malicious command detected"));
        }
        VerificationOutcome::Allowed(_) => {
            panic!("Expected Rejected, got Allowed");
        }
        VerificationOutcome::FallbackRequired(reason) => {
            // FallbackRequired also means the call won't proceed automatically,
            // which is acceptable for a malicious call test.
            panic!("Expected Rejected, got FallbackRequired: {}", reason);
        }
    }
}
