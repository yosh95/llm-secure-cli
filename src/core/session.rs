use crate::llm::base::LlmClient;
use crate::llm::models::{DataSource, MessagePart, Role};
use crate::security::audit::AuditEntry;
use crate::security::merkle_anchor::SessionAnchorManager;
use serde_json;
use std::sync::{Arc, OnceLock};
use tokio::sync::watch;
use uuid;

use crate::core::context::AppContext;

pub mod input_handler;
pub mod phase1_static;
pub mod phase2_verification;
pub mod phase3_execution;
pub mod processor;
pub mod tool_executor;

// ── Global Ctrl+C handler ────────────────────────────────────────────────
//
// On Unix, a persistent `tokio::signal::unix::signal(SIGINT)` listener is
// installed ONCE — the signal is captured by the OS-level sigaction and
// forwarded to an async receiver.  Repeated `ctrl_c().await` patterns
// create and destroy registrations in the global signal_hook_registry,
// which can cause signals to be silently dropped.
//
// On Windows, `tokio::signal::ctrl_c()` is used instead.
//
// All concurrent operations (LLM API call, verifier wait, tool execution)
// use independent `watch::Receiver`s from the same sender — no duplicate
// OS signal registrations.

static CANCEL_SENDER: OnceLock<watch::Sender<u64>> = OnceLock::new();

/// Ensure the global SIGINT listener is running (exactly once per process).
fn ensure_global_cancel_handler() {
    CANCEL_SENDER.get_or_init(|| {
        let (tx, _rx) = watch::channel(0u64);
        let tx2 = tx.clone();
        tokio::spawn(async move {
            // On Unix, register a persistent SIGINT listener ONCE
            // (avoids the re-registration race in `loop { ctrl_c().await }`).
            #[cfg(unix)]
            let mut stream: tokio::signal::unix::Signal =
                match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!(
                            "Failed to register persistent SIGINT handler: {e}; falling back"
                        );
                        // Pseudo-stream that calls ctrl_c() in a loop as fallback.
                        loop {
                            tokio::signal::ctrl_c().await.ok();
                            tracing::debug!("SIGINT received (fallback) — notifying watchers");
                            tx2.send_modify(|v| *v += 1);
                        }
                    }
                };

            loop {
                // Wait for the next SIGINT signal.
                #[cfg(unix)]
                stream.recv().await;
                #[cfg(not(unix))]
                tokio::signal::ctrl_c().await.ok();

                tracing::debug!("SIGINT received — notifying session cancellation watchers");
                tx2.send_modify(|v| *v += 1);
            }
        });
        tx
    });
}

/// A lightweight cancellation token backed by the global SIGINT handler.
///
/// Each `.receiver()` returns a fresh `watch::Receiver<u64>` that will
/// complete on the *next* Ctrl+C.  Multiple concurrent receivers (LLM call,
/// verifier, tool execution) all share the same single OS signal registration.
/// The handler runs in a loop and increments a counter on each Ctrl+C,
/// so cancellation works for an arbitrary number of sequential interruptions.
#[derive(Clone)]
pub struct SessionCancel;

impl SessionCancel {
    /// Create a new token — idempotent w.r.t. the global handler.
    /// Safe to call multiple times; the background task starts at most once.
    #[must_use]
    pub fn new() -> Self {
        ensure_global_cancel_handler();
        Self
    }

    /// Return a receiver that completes on the NEXT Ctrl+C.
    /// Each call is independent; the returned receiver starts fresh so its
    /// first `.changed()` always waits for a future signal.
    #[must_use]
    #[allow(clippy::expect_used)]
    pub fn receiver(&self) -> watch::Receiver<u64> {
        CANCEL_SENDER
            .get()
            .expect("SessionCancel::new() must have been called")
            .subscribe()
    }
}

impl Default for SessionCancel {
    fn default() -> Self {
        Self::new()
    }
}

/// A session that is actively running and has an initialized LLM client.
pub struct ActiveSession {
    pub client: Box<dyn LlmClient>,
    pub ctx: Arc<AppContext>,
    pub intent: String,
    pub pending_data: Vec<DataSource>,
    pub trace_id: String,
    pub audit_entries: Vec<AuditEntry>,
    pub total_usage: crate::llm::models::Usage,
    /// Shared cancellation token for Ctrl+C.  The single background listener
    /// (started once per process via `CANCEL_SENDER`) broadcasts to all
    /// concurrent operations through independent `watch::Receiver`s.
    pub cancel_token: SessionCancel,
    /// Set to true after `finalize_audit` has run once — prevents double-anchoring
    /// whether the session is closed via `close()` or via `Drop`.
    audit_finalized: bool,
}

/// A session that has been closed or failed to initialize.
pub struct ClosedSession {
    pub trace_id: String,
    pub audit_entries: Vec<AuditEntry>,
}

impl Drop for ActiveSession {
    fn drop(&mut self) {
        if !self.audit_finalized {
            self.finalize_audit();
        }
    }
}

impl ActiveSession {
    pub fn new(client: Box<dyn LlmClient>, ctx: Arc<AppContext>) -> anyhow::Result<Self> {
        let trace_id = format!("sess-{}", uuid::Uuid::new_v4().to_string().replace('-', ""));
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
            cancel_token: SessionCancel::new(),
            audit_finalized: false,
        })
    }

    /// Consumes the `ActiveSession` and returns a `ClosedSession`.
    ///
    /// This is the preferred way to close a session.  If `close()` is *not*
    /// called, `Drop` will still anchor the audit trail as a safety net.
    #[must_use]
    pub fn close(mut self) -> ClosedSession {
        self.finalize_audit();
        ClosedSession {
            trace_id: self.trace_id.clone(),
            audit_entries: std::mem::take(&mut self.audit_entries),
        }
    }

    /// Anchor the accumulated audit entries via the Merkle-anchor manager.
    ///
    /// Idempotent: only the first call has an effect; subsequent calls are
    /// no-ops.  This allows both `close()` and `Drop` to call this method
    /// without risk of double-anchoring.
    fn finalize_audit(&mut self) {
        if self.audit_finalized {
            return;
        }
        self.audit_finalized = true;

        // Purge the in-memory passphrase cache so that secrets are not
        // retained in memory after the session ends.
        crate::security::key_storage::purge_passphrase_cache();

        if self.audit_entries.is_empty() {
            return;
        }

        let entries_val: Vec<_> = self
            .audit_entries
            .iter()
            .filter_map(|e| serde_json::to_value(e).ok())
            .collect();

        if entries_val.is_empty() {
            return;
        }

        match SessionAnchorManager::create_anchor(&self.trace_id, Some(entries_val)) {
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(
                    trace_id = %self.trace_id,
                    error = %e,
                    "Failed to anchor audit trail for session"
                );
            }
        }
    }

    #[must_use]
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
        let should_remove = state.conversation.last().is_some_and(|msg| {
            (msg.role == Role::Assistant || msg.role == Role::Model)
                && msg
                    .parts
                    .iter()
                    .any(|part| matches!(part, MessagePart::Part(cp) if cp.function_call.is_some()))
        });

        if should_remove {
            // Remove the Assistant message with unanswered tool calls
            state.conversation.pop();
            // Also remove the preceding User message from the same turn to avoid
            // orphaned user input that would confuse the LLM on session reload.
            if let Some(last) = state.conversation.last()
                && last.role == Role::User
            {
                state.conversation.pop();
            }
        }

        // Auto-save the conversation up to this point so that Ctrl+C during
        // a ReAct loop (API call, verification, or tool execution) does not
        // cause the entire conversation to be lost.  This complements the
        // auto_save call in process_and_print() which only fires on a clean
        // exit (no more tool calls).  Saving here is safe because:
        //   - If no Assistant was removed and save is redundant → same content
        //   - If Assistant+User were removed → clean state after the last
        //     completed turn, which is exactly what the user expects
        //   - The call is idempotent (writes over the previous file) and the
        //     resulting file is always parseable as a valid SessionFile.
        crate::utils::session_store::auto_save(self);
    }
}
