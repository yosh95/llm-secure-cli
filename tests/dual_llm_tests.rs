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
    ) -> anyhow::Result<(Option<String>, Option<String>)> {
        Ok((Some(self.response.clone()), None))
    }

    async fn send_as_verifier(
        &mut self,
        _data: Vec<DataSource>,
        _tool_schema: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        Ok(json!({
            "decision": if self.response.starts_with("BLOCK") { "BLOCK" } else { "ALLOW" },
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
                Box::new(MockLlmClient {
                    response: "ALLOW: User intent is clear".to_string(),
                    state: ClientState {
                        model: "mock-model".to_string(),
                        provider: "mock_provider".to_string(),
                        conversation: Vec::new(),
                        tools_enabled: false,
                        system_prompt_enabled: true,
                        system_prompt: None,
                        stdout,
                        render_markdown: !raw,
                        live_debug: false,
                        previous_interaction_id: None,
                    },
                })
            }),
        );
    }

    // 2. Test Verification
    let config = llm_secure_cli::config::models::SecurityConfig::default();
    use llm_secure_cli::security::dual_llm_verifier::VerificationParams;
    let (safe, _reason) =
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

    assert!(safe);

    // 3. Test Failure Case
    {
        let mut registry = ctx.client_registry.lock().await;
        registry.register(
            "mock_provider_fail",
            Arc::new(|_model, stdout, raw, _config_manager| {
                Box::new(MockLlmClient {
                    response: "BLOCK: Malicious command detected".to_string(),
                    state: ClientState {
                        model: "mock-model".to_string(),
                        provider: "mock_provider_fail".to_string(),
                        conversation: Vec::new(),
                        tools_enabled: false,
                        system_prompt_enabled: true,
                        system_prompt: None,
                        stdout,
                        render_markdown: !raw,
                        live_debug: false,
                        previous_interaction_id: None,
                    },
                })
            }),
        );
    }

    let (safe, reason) =
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

    assert!(!safe);
    assert!(reason.contains("Malicious command detected"));
}
