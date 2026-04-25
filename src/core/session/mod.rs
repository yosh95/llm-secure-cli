use crate::llm::base::LlmClient;
use crate::llm::models::{ContentPart, DataSource, Message, MessagePart, Role};
use crate::security::audit::AuditEntry;
use crate::security::merkle_anchor::SessionAnchorManager;
use crate::security::verification_cache::VerificationCache;
use serde_json;
use std::collections::HashMap;
use uuid;

pub mod input_handler;
pub mod processor;
pub mod tool_executor;

pub struct ChatSession {
    pub client: Box<dyn LlmClient>,
    pub intent: String,
    pub pending_data: Vec<DataSource>,
    pub trace_id: String,
    pub audit_entries: Vec<AuditEntry>,
    pub verification_cache: VerificationCache,
}

impl Drop for ChatSession {
    fn drop(&mut self) {
        let entries_val = self
            .audit_entries
            .iter()
            .map(|e| serde_json::to_value(e).unwrap())
            .collect::<Vec<_>>();

        if !entries_val.is_empty() {
            let _ = SessionAnchorManager::create_anchor(&self.trace_id, Some(entries_val));
        }
    }
}

impl ChatSession {
    pub fn new(client: Box<dyn LlmClient>) -> Self {
        let trace_id = format!("sess-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        let user_id = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());

        let entry = crate::security::audit::log_audit_and_return(
            "session_start",
            "session",
            serde_json::json!({}),
            None,
            None,
            None,
            Some(&serde_json::json!({
                "trace_id": trace_id,
                "model": client.get_state().model,
                "user_id": user_id
            })),
        );

        Self {
            client,
            intent: String::new(),
            pending_data: Vec::new(),
            trace_id,
            audit_entries: entry.into_iter().collect(),
            verification_cache: VerificationCache::new(),
        }
    }

    /// Create a dummy empty session (internal use only for swapping)
    pub fn new_empty() -> Self {
        // This is a minimal client that does nothing.
        // It exists solely so that `std::mem::replace` has a valid placeholder
        // to swap in while the real session is being dropped. None of its
        // methods should ever be called after the swap.
        struct DummyClient {
            state: crate::llm::models::ClientState,
        }
        #[async_trait::async_trait]
        impl crate::llm::base::LlmClient for DummyClient {
            fn get_state(&self) -> &crate::llm::models::ClientState {
                &self.state
            }
            fn get_state_mut(&mut self) -> &mut crate::llm::models::ClientState {
                &mut self.state
            }
            fn get_config_section(&self) -> &str {
                "dummy"
            }
            async fn send(
                &mut self,
                _: Vec<crate::llm::models::DataSource>,
            ) -> anyhow::Result<(Option<String>, Option<String>)> {
                Ok((None, None))
            }
            async fn send_as_verifier(
                &mut self,
                _: Vec<crate::llm::models::DataSource>,
                _: serde_json::Value,
            ) -> anyhow::Result<serde_json::Value> {
                Ok(serde_json::json!({}))
            }
        }

        Self {
            client: Box::new(DummyClient {
                state: crate::llm::models::ClientState {
                    model: String::new(),
                    provider: String::new(),
                    conversation: Vec::new(),
                    tools_enabled: false,
                    system_prompt_enabled: false,
                    system_prompt: None,
                    stdout: false,
                    render_markdown: false,
                    live_debug: false,
                },
            }),
            intent: String::new(),
            pending_data: Vec::new(),
            trace_id: "dummy".to_string(),
            audit_entries: Vec::new(),
            verification_cache: VerificationCache::new(),
        }
    }

    pub fn switch_client(&mut self, mut new_client: Box<dyn LlmClient>) {
        {
            let old_state = self.client.get_state();
            let new_state = new_client.get_state_mut();
            new_state.conversation = old_state.conversation.clone();
            new_state.live_debug = old_state.live_debug;
            if new_state.tools_enabled {
                new_state.tools_enabled = old_state.tools_enabled;
            }
            new_state.system_prompt_enabled = old_state.system_prompt_enabled;
        }
        self.client = new_client;
    }

    pub(crate) fn handle_interruption(&mut self) {
        let state = self.client.get_state_mut();
        let last_msg = state.conversation.last().cloned();
        if let Some(msg) = last_msg
            && (msg.role == Role::Assistant || msg.role == Role::Model)
        {
            let mut has_unanswered_tools = false;
            for part in &msg.parts {
                if let MessagePart::Part(cp) = part
                    && cp.function_call.is_some()
                {
                    has_unanswered_tools = true;
                    break;
                }
            }

            if has_unanswered_tools {
                let mut tool_results = Vec::new();
                for part in &msg.parts {
                    if let MessagePart::Part(cp) = part
                        && let Some(fc) = &cp.function_call
                    {
                        let name = fc.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let id = fc.get("id").and_then(|v| v.as_str()).unwrap_or("");

                        let mut fr = HashMap::new();
                        fr.insert("id".to_string(), serde_json::json!(id));
                        fr.insert("name".to_string(), serde_json::json!(name));
                        fr.insert(
                            "response".to_string(),
                            serde_json::json!("Error: Interrupted by user."),
                        );

                        tool_results.push(MessagePart::Part(ContentPart {
                            text: None,
                            inline_data: None,
                            function_call: None,
                            function_response: Some(fr),
                            thought: None,
                            thought_signature: None,
                            is_diagnostic: false,
                        }));
                    }
                }
                state.conversation.push(Message {
                    role: Role::Tool,
                    parts: tool_results,
                });
            }
        }
    }
}
