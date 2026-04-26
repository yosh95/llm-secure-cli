use async_trait::async_trait;
use llm_secure_cli::llm::base::LlmClient;
use llm_secure_cli::llm::models::{ClientState, DataSource};
use llm_secure_cli::llm::registry::CLIENT_REGISTRY;
use serde_json::json;

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
    async fn send(
        &mut self,
        _data: Vec<DataSource>,
    ) -> anyhow::Result<(Option<String>, Option<String>)> {
        Ok((Some(self.response.clone()), None))
    }

    async fn send_as_verifier(
        &mut self,
        _data: Vec<DataSource>,
        _tool_schema: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        Ok(json!({
            "safe": !self.response.starts_with("BLOCK"),
            "reason": self.response.clone()
        }))
    }
}

#[tokio::test]
async fn test_dual_llm_verification_logic() {
    // 1. Register Mock Client (Success case)
    {
        let mut registry = CLIENT_REGISTRY.lock().unwrap();
        registry.register("mock_provider", |_model, stdout, raw| {
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
                },
            })
        });
    }

    // 2. Test Verification
    let (safe, _reason) = llm_secure_cli::security::dual_llm_verifier::verify_tool_call_full(
        "I want to list files",
        "list_files_in_directory",
        &json!({"directory": "."}),
        None,
        Some("mock_provider".to_string()),
        Some("mock-model".to_string()),
    )
    .await;

    assert!(safe);

    // 3. Test Failure Case
    {
        let mut registry = CLIENT_REGISTRY.lock().unwrap();
        registry.register("mock_provider_fail", |_model, stdout, raw| {
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
                },
            })
        });
    }

    let (safe, reason) = llm_secure_cli::security::dual_llm_verifier::verify_tool_call_full(
        "I want to delete everything",
        "execute_command",
        &json!({"command": "rm", "args": ["-rf", "/"]}),
        None,
        Some("mock_provider_fail".to_string()),
        Some("mock-model".to_string()),
    )
    .await;

    assert!(!safe);
    assert!(reason.contains("Malicious command detected"));
}
