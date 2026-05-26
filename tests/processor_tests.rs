use async_trait::async_trait;
use llm_secure_cli::core::context::AppContext;
use llm_secure_cli::core::session::ActiveSession;
use llm_secure_cli::llm::base::LlmClient;
use llm_secure_cli::llm::models::{
    ClientState, ContentPart, DataSource, Message, MessagePart, Role,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Once;

static INIT: Once = Once::new();

use llm_secure_cli::cli::ui::{ConfirmResult, UserInterface};

struct MockUi {
    pub confirmed: bool,
}

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
        if self.confirmed {
            Some(ConfirmResult::Yes)
        } else {
            Some(ConfirmResult::No)
        }
    }
    async fn ask_confirm_simple(&self, _prompt: &str) -> Option<ConfirmResult> {
        if self.confirmed {
            Some(ConfirmResult::Yes)
        } else {
            Some(ConfirmResult::No)
        }
    }
}

fn setup_test_env() {
    INIT.call_once(|| {
        let tmp = tempfile::tempdir()
            .expect("Failed to create temp dir")
            .keep();
        llm_secure_cli::consts::init_base_dir(Some(tmp));
    });
}

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
    ) -> anyhow::Result<llm_secure_cli::llm::models::LlmResponse> {
        if self.call_count >= self.responses.len() {
            return Ok(llm_secure_cli::llm::models::LlmResponse::default());
        }
        let (text, _thought, parts) = self.responses[self.call_count].clone();
        self.call_count += 1;

        if let Some(p) = parts {
            self.state.conversation.push(Message {
                role: Role::Assistant,
                parts: p.clone(),
            });

            // If it's a tool call, we need to populate LlmResponse properly
            for part in p {
                if let MessagePart::Part(cp) = part
                    && let Some(fc) = cp.function_call
                {
                    return Ok(llm_secure_cli::llm::models::LlmResponse {
                        content: text,
                        tool_name: fc
                            .get("name")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string()),
                        ..Default::default()
                    });
                }
            }
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

        Ok(llm_secure_cli::llm::models::LlmResponse {
            content: text,
            tool_name: None,
            ..Default::default()
        })
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

#[tokio::test(flavor = "multi_thread")]
async fn test_processor_tool_execution_flow() {
    setup_test_env();
    // 1. Setup Config for Auto-approval
    let ui = Arc::new(MockUi { confirmed: true });
    let ctx = AppContext::new(ui);
    let mut config = (*ctx
        .config_manager
        .get_config()
        .expect("Failed to get config"))
    .clone();
    config.security.security_level = llm_secure_cli::config::models::SecurityLevel::Standard;

    config.security.verifier_enabled = Some(true);
    config.security.verifier_provider = "mock".to_string();
    let _ = ctx.config_manager.set_config(config);
    let ctx = Arc::new(ctx);

    // 2. Register Mock Client in Registry for Verifier
    {
        let mut registry = ctx.client_registry.lock().await;
        registry.register(
            "mock",
            Arc::new(|_model, stdout, raw, _config_manager| {
                Ok(Box::new(MockProcessorClient {
                    state: ClientState {
                        model: "mock-model".to_string(),
                        provider: "mock".to_string(),
                        conversation: Vec::new(),
                        tools_enabled: false,
                        system_prompt_enabled: true,
                        system_prompt: None,
                        stdout,
                        render_markdown: !raw,
                    },
                    responses: vec![],
                    call_count: 0,
                }))
            }),
        );
    }

    // 3. Setup Mock Client for the main session
    let mut fc_map = HashMap::new();
    fc_map.insert("name".to_string(), json!("list_files"));
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
        },
        responses: vec![
            (None, None, Some(vec![tool_call_part])),
            (Some("Done".to_string()), None, None),
        ],
        call_count: 0,
    };

    let mut session =
        ActiveSession::new(Box::new(mock_client), ctx).expect("Failed to create session");
    session.intent = "test".to_string();
    session.trace_id = "test-trace".to_string();

    // 4. Execute
    let result = session.process_and_print(vec![]).await;
    assert!(result.is_ok());

    // 5. Verify
    let state = session.client.get_state();
    assert_eq!(state.conversation.len(), 3);
    assert_eq!(state.conversation[1].role, Role::Tool);

    // Check audit logs
    assert!(!session.audit_entries.is_empty());
    assert!(session.audit_entries.iter().any(|e| e.tool == "list_files"));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_processor_pqc_blocking_in_high_security() {
    setup_test_env();
    // 1. Setup Config with High Security Level
    let ui = Arc::new(MockUi { confirmed: true });
    let ctx = AppContext::new(ui);
    let mut config = (*ctx
        .config_manager
        .get_config()
        .expect("Failed to get config"))
    .clone();
    config.security.security_level = llm_secure_cli::config::models::SecurityLevel::High;

    let _ = ctx.config_manager.set_config(config);
    let ctx = Arc::new(ctx);

    // 2. Setup Mock Client with a tool call
    let mut fc_map = HashMap::new();
    fc_map.insert("name".to_string(), json!("list_files"));
    fc_map.insert("arguments".to_string(), json!({"directory": "."}));
    fc_map.insert("id".to_string(), json!("call_456"));

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
            stdout: false, // Suppress output to keep test logs clean
            render_markdown: false,
        },
        responses: vec![
            (None, None, Some(vec![tool_call_part])),
            (Some("Done".to_string()), None, None),
        ],
        call_count: 0,
    };

    let mut session =
        ActiveSession::new(Box::new(mock_client), ctx).expect("Failed to create session");
    session.intent = "test-high-security".to_string();

    // 3. Execute - This should trigger the PQC Error
    let _ = session.process_and_print(vec![]).await;

    // 4. Verify - In High Security mode without a key, the audit entry is still
    // persisted (with INTEGRITY_FAILURE status) for forensic traceability.
    assert!(
        !session.audit_entries.is_empty(),
        "Audit entries should NOT be empty — failed entries are persisted for forensic traceability"
    );
    assert!(
        session.audit_entries.iter().any(|e| matches!(
            e.status,
            llm_secure_cli::security::audit::AuditStatus::IntegrityFailure(_)
        )),
        "Audit entry should have INTEGRITY_FAILURE status"
    );
}
