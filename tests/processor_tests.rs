use async_trait::async_trait;
use llm_secure_cli::core::context::AppContext;
use llm_secure_cli::core::session::ChatSession;
use llm_secure_cli::llm::base::LlmClient;
use llm_secure_cli::llm::models::{
    ClientState, ContentPart, DataSource, Message, MessagePart, Role,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

type MockResponse = (Option<String>, Option<String>, Option<Vec<MessagePart>>);

struct MockProcessorClient {
    state: ClientState,
    responses: Vec<MockResponse>,
    call_count: usize,
}

#[async_trait]
impl LlmClient for MockProcessorClient {
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
        if self.call_count >= self.responses.len() {
            return Ok((None, None));
        }
        let (text, thought, parts) = self.responses[self.call_count].clone();
        self.call_count += 1;

        if let Some(p) = parts {
            self.state.conversation.push(Message {
                role: Role::Assistant,
                parts: p,
            });
        } else if let Some(t) = &text {
            self.state.conversation.push(Message {
                role: Role::Assistant,
                parts: vec![MessagePart::Part(Box::new(ContentPart {
                    text: Some(t.clone()),
                    is_diagnostic: false,
                    ..Default::default()
                }))],
            });
        }

        Ok((text, thought))
    }

    async fn send_as_verifier(
        &mut self,
        _data: Vec<DataSource>,
        _tool_schema: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        Ok(json!({
            "decision": "ALLOW",
            "reason": "Mocked allow"
        }))
    }
}

#[tokio::test]
async fn test_processor_tool_execution_flow() {
    // 1. Setup Config for Auto-approval
    let ctx = AppContext::new();
    let mut config = ctx.config_manager.get_config();
    config.security.auto_approval_level = Some("low".to_string());
    config.security.low_risk_tools = vec!["list_files_in_directory".to_string()];
    config.security.dual_llm_verification = Some(true);
    config.security.dual_llm_provider = "mock".to_string();
    ctx.config_manager.set_config(config);
    let ctx = Arc::new(ctx);

    // 2. Register Mock Client in Registry for Verifier
    {
        let mut registry = ctx.client_registry.lock().await;
        registry.register(
            "mock",
            Arc::new(|_model, stdout, raw, _config_manager| {
                Box::new(MockProcessorClient {
                    state: ClientState {
                        model: "mock-model".to_string(),
                        provider: "mock".to_string(),
                        conversation: Vec::new(),
                        tools_enabled: false,
                        system_prompt_enabled: true,
                        system_prompt: None,
                        stdout,
                        render_markdown: !raw,
                        live_debug: false,
                        previous_interaction_id: None,
                    },
                    responses: vec![],
                    call_count: 0,
                })
            }),
        );
    }

    // 3. Setup Mock Client for the main session
    let mut fc_map = HashMap::new();
    fc_map.insert("name".to_string(), json!("list_files_in_directory"));
    fc_map.insert("arguments".to_string(), json!({"directory": "."}));
    fc_map.insert("id".to_string(), json!("call_123"));

    let tool_call_part = MessagePart::Part(Box::new(ContentPart {
        function_call: Some(fc_map),
        is_diagnostic: false,
        ..Default::default()
    }));

    let mock_client = MockProcessorClient {
        state: ClientState {
            model: "mock-model".to_string(),
            provider: "mock".to_string(),
            conversation: Vec::new(),
            tools_enabled: true,
            system_prompt_enabled: false,
            system_prompt: None,
            stdout: false,
            render_markdown: false,
            live_debug: false,
            previous_interaction_id: None,
        },
        responses: vec![
            (None, None, Some(vec![tool_call_part])),
            (Some("Done".to_string()), None, None),
        ],
        call_count: 0,
    };

    let mut session = ChatSession {
        client: Box::new(mock_client),
        ctx,
        intent: "test".to_string(),
        pending_data: Vec::new(),
        trace_id: "test-trace".to_string(),
        audit_entries: Vec::new(),
    };

    // 4. Execute
    let result = session.process_and_print(vec![]).await;
    assert!(result.is_ok());

    // 5. Verify
    let state = session.client.get_state();
    assert_eq!(state.conversation.len(), 3);
    assert_eq!(state.conversation[1].role, Role::Tool);

    // Check audit logs
    assert!(!session.audit_entries.is_empty());
    assert!(
        session
            .audit_entries
            .iter()
            .any(|e| e.tool == "list_files_in_directory")
    );
}
