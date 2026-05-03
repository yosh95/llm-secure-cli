use crate::llm::base::LlmClient;
use crate::llm::models::{ContentPart, DataSource, Message, MessagePart, Role};
use crate::security::audit::AuditEntry;
use crate::security::merkle_anchor::SessionAnchorManager;
use serde_json;
use std::collections::HashMap;
use uuid;

use crate::core::context::AppContext;
use std::sync::Arc;

pub mod input_handler;
pub mod processor;
pub mod tool_executor;

pub struct ChatSession {
    pub client: Option<Box<dyn LlmClient>>,
    pub ctx: Arc<AppContext>,
    pub intent: String,
    pub pending_data: Vec<DataSource>,
    pub trace_id: String,
    pub audit_entries: Vec<AuditEntry>,
}

impl Drop for ChatSession {
    fn drop(&mut self) {
        let entries_val = self
            .audit_entries
            .iter()
            .filter_map(|e| serde_json::to_value(e).ok())
            .collect::<Vec<_>>();

        if !entries_val.is_empty() {
            let _ = SessionAnchorManager::create_anchor(&self.trace_id, Some(entries_val));
        }
    }
}

impl ChatSession {
    pub fn new(client: Box<dyn LlmClient>, ctx: Arc<AppContext>) -> anyhow::Result<Self> {
        let trace_id = format!("sess-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        let user_id = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());

        let config = ctx.config_manager.get_config()?;
        let context_val = serde_json::json!({
            "trace_id": trace_id,
            "model": client.get_state().model,
            "user_id": user_id
        });
        let entry = crate::security::audit::log_audit_and_return(
            crate::security::audit::AuditParams {
                event_type: "session_start",
                tool_name: "session",
                args: serde_json::json!({}),
                output: None,
                exit_code: None,
                error: None,
                context: Some(&context_val),
                config: &config,
            },
            None,
        );

        Ok(Self {
            client: Some(client),
            ctx,
            intent: String::new(),
            pending_data: Vec::new(),
            trace_id,
            audit_entries: entry.into_iter().collect(),
        })
    }

    /// Create an empty placeholder session
    pub fn empty(ctx: Arc<AppContext>) -> Self {
        Self {
            client: None,
            ctx,
            intent: String::new(),
            pending_data: Vec::new(),
            trace_id: "none".to_string(),
            audit_entries: Vec::new(),
        }
    }

    pub fn get_client(&self) -> anyhow::Result<&(dyn LlmClient + '_)> {
        match self.client.as_ref() {
            Some(b) => Ok(b.as_ref()),
            None => Err(anyhow::anyhow!("ChatSession accessed after being cleared")),
        }
    }

    pub fn get_client_mut(&mut self) -> anyhow::Result<&mut (dyn LlmClient + '_)> {
        match self.client.as_mut() {
            Some(b) => Ok(b.as_mut()),
            None => Err(anyhow::anyhow!("ChatSession accessed after being cleared")),
        }
    }

    pub fn switch_client(&mut self, mut new_client: Box<dyn LlmClient>) {
        if let Some(old_client) = &self.client {
            let old_state = old_client.get_state();
            let new_state = new_client.get_state_mut();
            new_state.conversation = old_state.conversation.clone();
            new_state.live_debug = old_state.live_debug;
            if new_state.tools_enabled {
                new_state.tools_enabled = old_state.tools_enabled;
            }
            new_state.system_prompt_enabled = old_state.system_prompt_enabled;
        }
        self.client = Some(new_client);
    }

    pub(crate) fn handle_interruption(&mut self) {
        if let Some(client) = &mut self.client {
            let state = client.get_state_mut();
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

                            tool_results.push(MessagePart::Part(Box::new(ContentPart {
                                function_response: Some(fr),
                                is_diagnostic: false,
                                ..Default::default()
                            })));
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
}
