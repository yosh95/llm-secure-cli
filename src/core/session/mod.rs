use crate::llm::base::LlmClient;
use crate::llm::models::{DataSource, MessagePart, Role};
use crate::security::audit::AuditEntry;
use crate::security::merkle_anchor::SessionAnchorManager;
use serde_json;
use uuid;

use crate::core::context::AppContext;
use std::sync::Arc;

pub mod input_handler;
pub mod phase1_static;
pub mod phase2_approval;
pub mod phase3_verifier;
pub mod phase4_execution;
pub mod processor;
pub mod tool_executor;

/// A session that is actively running and has an initialized LLM client.
pub struct ActiveSession {
    pub client: Box<dyn LlmClient>,
    pub ctx: Arc<AppContext>,
    pub intent: String,
    pub pending_data: Vec<DataSource>,
    pub trace_id: String,
    pub audit_entries: Vec<AuditEntry>,
    pub total_usage: crate::llm::models::Usage,
}

/// A session that has been closed or failed to initialize.
pub struct ClosedSession {
    pub trace_id: String,
    pub audit_entries: Vec<AuditEntry>,
}

impl Drop for ActiveSession {
    fn drop(&mut self) {
        self.finalize_audit();
    }
}

impl ActiveSession {
    pub fn new(client: Box<dyn LlmClient>, ctx: Arc<AppContext>) -> anyhow::Result<Self> {
        let trace_id = format!("sess-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        let user_id = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());

        let config = ctx.config_manager.get_config()?;
        let context_val = serde_json::json!({
            "trace_id": trace_id,
            "model": client.get_state().model,
            "provider": client.get_state().provider,
            "user_id": user_id
        });
        let entry =
            crate::security::audit::AuditParams::builder("session_start", "session", &config)
                .context(&context_val)
                .log_and_return(None);

        Ok(Self {
            client,
            ctx,
            intent: String::new(),
            pending_data: Vec::new(),
            trace_id,
            audit_entries: entry.into_iter().collect(),
            total_usage: crate::llm::models::Usage::default(),
        })
    }

    /// Consumes the ActiveSession and returns a ClosedSession.
    pub fn close(mut self) -> ClosedSession {
        self.finalize_audit();
        ClosedSession {
            trace_id: self.trace_id.clone(),
            audit_entries: std::mem::take(&mut self.audit_entries),
        }
    }

    fn finalize_audit(&mut self) {
        let entries_val = self
            .audit_entries
            .iter()
            .filter_map(|e| serde_json::to_value(e).ok())
            .collect::<Vec<_>>();

        if !entries_val.is_empty() {
            let _ = SessionAnchorManager::create_anchor(&self.trace_id, Some(entries_val));
        }
    }

    pub fn get_client(&self) -> &(dyn LlmClient + '_) {
        self.client.as_ref()
    }

    pub fn get_client_mut(&mut self) -> &mut (dyn LlmClient + '_) {
        self.client.as_mut()
    }

    pub fn switch_client(&mut self, mut new_client: Box<dyn LlmClient>) {
        let old_state = self.client.get_state();
        let new_state = new_client.get_state_mut();
        new_state.conversation = old_state.conversation.clone();
        if new_state.tools_enabled {
            new_state.tools_enabled = old_state.tools_enabled;
        }
        new_state.system_prompt_enabled = old_state.system_prompt_enabled;
        self.client = new_client;
    }

    pub(crate) fn handle_interruption(&mut self) {
        let state = self.client.get_state_mut();
        // Remove the last assistant/model message if it contains unanswered tool calls.
        // Since the tools were never executed, injecting fake error results would
        // create an inconsistency between the conversation history and the audit log.
        // Simply removing the unactioned message is the cleanest approach:
        // the LLM will re-evaluate the conversation state on the next turn.
        let should_remove = state
            .conversation
            .last()
            .map(|msg| {
                (msg.role == Role::Assistant || msg.role == Role::Model)
                    && msg.parts.iter().any(
                        |part| matches!(part, MessagePart::Part(cp) if cp.function_call.is_some()),
                    )
            })
            .unwrap_or(false);

        if should_remove {
            state.conversation.pop();
        }
    }
}
