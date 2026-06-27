#![allow(clippy::unwrap_used, clippy::expect_used)]

//! # Audit Verification Tests
//!
//! These tests verify that **Verifier判定結果、拒否理由、HITLに回した結果、HITLのフィードバック**
//! are correctly recorded in the audit log.
//!
//! ## Test strategy
//!
//! We use a full `ActiveSession` + `process_and_print` flow (same as
//! `processor_tests.rs`) to exercise `phase2_verification` end-to-end, then
//! inspect the on-disk audit log file.
//!
//! Because `phase2_verification` is `pub(crate)`, we cannot call it directly
//! from integration tests — the `process_and_print` entry point is the
//! intended public API.
//!
//! Each test creates its own temporary directory to avoid interference.

use llm_secure_cli::cli::ui::{ConfirmResult, UserInterface};
use llm_secure_cli::core::context::AppContext;
use llm_secure_cli::core::session::ActiveSession;
use llm_secure_cli::llm::base::LlmClient;
use llm_secure_cli::llm::models::{
    ClientState, ContentPart, DataSource, LlmResponse, Message, MessagePart, Role,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Global mutex to serialize audit log access across parallel tests.
/// The audit log is file-based and shared (OnceLock), so parallel writes
/// lead to racy assertions.  This mutex ensures one test at a time.
static AUDIT_LOG_MUTEX: Mutex<()> = Mutex::new(());

/// Initialize the test environment (called once, subsequent calls are no-ops).
fn create_test_env() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let tmp = tempfile::tempdir()
            .expect("Failed to create temp dir")
            .keep();
        llm_secure_cli::consts::init_base_dir(Some(tmp));
    });
}

/// A mock UI that records all calls and returns a pre-configured confirmation result.
struct TestUi {
    /// If `Some(true)`, ask_confirm returns Yes.
    /// If `Some(false)`, ask_confirm returns No (or Feedback if feedback_text is set).
    pub confirmed: Option<bool>,
    /// If `Some(text)`, ask_confirm returns Feedback(text) when confirmed is false.
    /// If `Some("")`, it returns Feedback("") which means "rejected without typed feedback".
    pub feedback_text: Option<String>,
    #[allow(dead_code)]
    pub messages: Vec<String>,
}

impl TestUi {
    fn always_yes() -> Self {
        Self {
            confirmed: Some(true),
            feedback_text: None,
            messages: Vec::new(),
        }
    }

    fn always_no() -> Self {
        Self {
            // Feedback("") triggers handle_rejection_feedback(Some(""))
            // which returns "Error: Cancelled by user." WITHOUT calling get_user_input()
            confirmed: Some(false),
            feedback_text: Some(String::new()),
            messages: Vec::new(),
        }
    }

    fn reject_with_feedback(feedback: &str) -> Self {
        Self {
            confirmed: Some(false),
            feedback_text: Some(feedback.to_string()),
            messages: Vec::new(),
        }
    }
}

impl UserInterface for TestUi {
    fn print_block(&self, _c: &str, _t: Option<&str>) {}
    fn print_tool_call(&self, _n: &str, _a: &serde_json::Value) {}
    fn print_tool_call_direct(&self, _n: &str, _a: &serde_json::Value) {}
    fn print_tool_result(&self, _r: &str) {}
    fn report_error(&self, _m: &str) {}
    fn report_info(&self, _m: &str) {}
    fn report_warning(&self, _m: &str) {}
    fn report_success(&self, _m: &str) {}
    fn ask_confirm(&self, _p: &str) -> Option<ConfirmResult> {
        self.confirmed.map(|y| {
            if y {
                ConfirmResult::Yes
            } else if let Some(ref fb) = self.feedback_text {
                ConfirmResult::Feedback(fb.clone())
            } else {
                ConfirmResult::No
            }
        })
    }
    fn ask_confirm_simple(&self, _p: &str) -> Option<ConfirmResult> {
        self.ask_confirm(_p)
    }
}

/// A mock LLM client used for the **main** session (not the verifier).
struct SessionMockClient {
    state: ClientState,
    call_count: usize,
}

impl LlmClient for SessionMockClient {
    fn get_state(&self) -> &ClientState {
        &self.state
    }
    fn get_state_mut(&mut self) -> &mut ClientState {
        &mut self.state
    }
    fn get_config_section(&self) -> &str {
        "mock"
    }
    fn send(
        &mut self,
        _data: Vec<DataSource>,
        _tool_schemas: Vec<serde_json::Value>,
    ) -> anyhow::Result<LlmResponse> {
        if self.call_count == 0 {
            self.call_count += 1;
            let mut fc = HashMap::new();
            fc.insert("name".to_string(), json!("execute_python"));
            fc.insert("arguments".to_string(), json!({"code": "print(1 + 1)"}));
            fc.insert("id".to_string(), json!("call_audit_test"));

            let part = MessagePart::Part(Box::new(ContentPart {
                function_call: Some(fc),
                is_diagnostic: false,
                ..Default::default()
            }));

            self.state.conversation.push(Message {
                role: Role::Assistant,
                parts: vec![part.clone()],
            });

            Ok(LlmResponse {
                content: None,
                tool_name: Some("execute_python".to_string()),
                ..Default::default()
            })
        } else {
            self.call_count += 1;
            let text = "Done.".to_string();
            self.state.conversation.push(Message {
                role: Role::Assistant,
                parts: vec![MessagePart::Part(Box::new(ContentPart {
                    text: Some(text.clone()),
                    is_diagnostic: false,
                    ..Default::default()
                }))],
            });
            Ok(LlmResponse {
                content: Some(text),
                tool_name: None,
                ..Default::default()
            })
        }
    }

    fn send_as_verifier(
        &mut self,
        _data: Vec<DataSource>,
        _tool_schema: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        Ok(json!({
            "decision": "ALLOW",
            "reason": "Mock verifier allow"
        }))
    }
}

/// A mock LLM client used **only** for the Verifier LLM.
///
/// Registered in the client registry so that `verify_tool_call_full`
/// can instantiate it when the verifier committee runs.
///
/// IMPORTANT: `send()` returns the raw text response (e.g. "DECISION: ALLOW\nREASON: ...")
/// which `parse_verifier_response` will parse. `send_as_verifier()` is NOT called
/// by the verifier flow — only `send()` is used.
struct VerifierMockClient {
    state: ClientState,
    response_text: String,
}

impl LlmClient for VerifierMockClient {
    fn get_state(&self) -> &ClientState {
        &self.state
    }
    fn get_state_mut(&mut self) -> &mut ClientState {
        &mut self.state
    }
    fn get_config_section(&self) -> &str {
        "mock_verifier"
    }
    fn send(
        &mut self,
        _data: Vec<DataSource>,
        _tool_schemas: Vec<serde_json::Value>,
    ) -> anyhow::Result<LlmResponse> {
        Ok(LlmResponse {
            content: Some(self.response_text.clone()),
            ..Default::default()
        })
    }

    fn send_as_verifier(
        &mut self,
        _data: Vec<DataSource>,
        _tool_schema: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        Ok(json!({
            "decision": if self.response_text.contains("ALLOW") { "ALLOW" } else { "BLOCK" },
            "reason": self.response_text.clone()
        }))
    }
}

/// Register a verifier mock client in the registry.
fn register_verifier_mock(ctx: &Arc<AppContext>, provider_name: &str, response_text: &str) {
    let mut registry = ctx.client_registry.lock().unwrap();
    let p_name = provider_name.to_string();
    let resp = response_text.to_string();
    registry.register(
        &p_name.clone(),
        Arc::new(move |_model, _stdout, _raw, _cfg| {
            Ok(Box::new(VerifierMockClient {
                state: ClientState {
                    model: "verifier-model".to_string(),
                    provider: p_name.clone(),
                    conversation: Vec::new(),
                    system_prompt: None,
                    stdout: false,
                    render_markdown: false,
                },
                response_text: resp.clone(),
            }) as Box<dyn LlmClient>)
        }),
    );
}

/// Helper: create a session, configure it, and run `process_and_print`.
/// Returns the session.
///
/// When `verifier_response` is `Some(...)`, the verifier is configured via
/// state.toml with `mock_verifier:verifier-model` and enabled.
/// When `None`, the verifier is disabled (no provider:model set, or disabled).
fn run_session(ui: TestUi, verifier_response: Option<&str>) -> ActiveSession {
    create_test_env();
    // Clear audit log to avoid cross-test contamination
    let log_path = llm_secure_cli::consts::audit_log_path();
    if log_path.exists() {
        std::fs::remove_file(&log_path).ok();
    }
    // Also clear the head-pointer cache
    let cache_path = llm_secure_cli::consts::audit_head_cache_path();
    if cache_path.exists() {
        std::fs::remove_file(&cache_path).ok();
    }

    let ui = Arc::new(ui);
    let ctx = AppContext::new(ui);

    // Standard security level (no PQC key required)
    {
        let config = (*ctx
            .config_manager
            .get_config()
            .expect("Failed to get config"))
        .clone();
        // SecurityLevel removed; always high equivalent
        let _ = ctx.config_manager.set_config(config);
    }

    if let Some(v_resp) = verifier_response {
        // Configure verifier via SecurityConfig (config.toml)
        {
            let config = (*ctx
                .config_manager
                .get_config()
                .expect("Failed to get config"))
            .clone();
            let mut config = config;
            config.security.verifier_committee = vec!["mock_verifier:verifier-model".to_string()];
            config.security.verifier_enabled = true;
            ctx.config_manager
                .set_config(config)
                .expect("Failed to update config");
        }

        let ctx = Arc::new(ctx);

        // Register the verifier mock
        register_verifier_mock(&ctx, "mock_verifier", v_resp);

        // Create session client
        let mock_client = SessionMockClient {
            state: ClientState {
                model: "mock-model".to_string(),
                provider: "mock".to_string(),
                conversation: Vec::new(),
                system_prompt: None,
                stdout: false,
                render_markdown: false,
            },
            call_count: 0,
        };

        let mut session =
            ActiveSession::new(Box::new(mock_client), ctx).expect("Failed to create session");
        session.intent = "test audit verification".to_string();

        let result = session.process_and_print(vec![]);
        result.expect("process_and_print should succeed");
        session
    } else {
        // Verifier disabled: explicitly disable in config
        {
            let config = (*ctx
                .config_manager
                .get_config()
                .expect("Failed to get config"))
            .clone();
            let mut config = config;
            config.security.verifier_committee = Vec::new();
            config.security.verifier_enabled = false;
            ctx.config_manager
                .set_config(config)
                .expect("Failed to update config");
        }

        let ctx = Arc::new(ctx);

        let mock_client = SessionMockClient {
            state: ClientState {
                model: "mock-model".to_string(),
                provider: "mock".to_string(),
                conversation: Vec::new(),
                system_prompt: None,
                stdout: false,
                render_markdown: false,
            },
            call_count: 0,
        };

        let mut session =
            ActiveSession::new(Box::new(mock_client), ctx).expect("Failed to create session");
        session.intent = "test audit verification".to_string();

        let result = session.process_and_print(vec![]);
        result.expect("process_and_print should succeed");
        session
    }
}

/// Read the audit log file and return all entries as a vector of JSON values.
fn read_audit_log() -> Vec<serde_json::Value> {
    let log_path = llm_secure_cli::consts::audit_log_path();
    if !log_path.exists() {
        return Vec::new();
    }
    let content = std::fs::read_to_string(&log_path).unwrap_or_default();
    content
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                None
            } else {
                serde_json::from_str(trimmed).ok()
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests: Verifier not configured (NoVerifier path)
// ---------------------------------------------------------------------------

#[test]
fn test_audit_no_verifier_human_approves() {
    let _lock = AUDIT_LOG_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _session = run_session(TestUi::always_yes(), None);

    let log_entries = read_audit_log();

    // DEBUG: Print all entries
    for (i, entry) in log_entries.iter().enumerate() {
        eprintln!(
            "DEBUG Entry {}: event_type={:?}, args={:?}",
            i,
            entry.get("event_type"),
            entry.get("args")
        );
    }

    // Check verifier_decision entries
    let verifier_decisions: Vec<_> = log_entries
        .iter()
        .filter(|e| e.get("event_type").and_then(|v| v.as_str()) == Some("verifier_decision"))
        .collect();
    assert!(
        !verifier_decisions.is_empty(),
        "Expected at least one verifier_decision entry, got {}",
        log_entries.len()
    );

    // Find the NoVerifier entry
    let no_verifier = verifier_decisions.iter().find(|e| {
        e.get("args")
            .and_then(|a| a.get("verdict"))
            .and_then(|v| v.as_str())
            == Some("NoVerifier")
    });
    assert!(no_verifier.is_some(), "Expected a NoVerifier verdict entry");

    // Check human_approval entry
    let approval_entries: Vec<_> = log_entries
        .iter()
        .filter(|e| e.get("event_type").and_then(|v| v.as_str()) == Some("human_approval"))
        .collect();
    assert!(
        !approval_entries.is_empty(),
        "Expected human_approval entry"
    );
    let approval = approval_entries.iter().find(|e| {
        e.get("args")
            .and_then(|a| a.get("result"))
            .and_then(|v| v.as_str())
            == Some("approved")
    });
    assert!(approval.is_some(), "Expected approved human_approval entry");
}

#[test]
fn test_audit_no_verifier_human_rejects() {
    let _lock = AUDIT_LOG_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _session = run_session(TestUi::always_no(), None);

    let log_entries = read_audit_log();

    // Check verifier_decision entry
    let no_verifier: Vec<_> = log_entries
        .iter()
        .filter(|e| {
            e.get("event_type").and_then(|v| v.as_str()) == Some("verifier_decision")
                && e.get("args")
                    .and_then(|a| a.get("verdict"))
                    .and_then(|v| v.as_str())
                    == Some("NoVerifier")
        })
        .collect();
    assert!(!no_verifier.is_empty(), "Expected NoVerifier entry");

    // Check human_approval entry with result=rejected
    let rejected: Vec<_> = log_entries
        .iter()
        .filter(|e| {
            e.get("event_type").and_then(|v| v.as_str()) == Some("human_approval")
                && e.get("args")
                    .and_then(|a| a.get("result"))
                    .and_then(|v| v.as_str())
                    == Some("rejected")
        })
        .collect();
    assert!(
        !rejected.is_empty(),
        "Expected rejected human_approval entry"
    );
}

#[test]
fn test_audit_no_verifier_human_rejects_with_feedback() {
    let _lock = AUDIT_LOG_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _session = run_session(
        TestUi::reject_with_feedback("This is unsafe, do not run"),
        None,
    );

    let log_entries = read_audit_log();

    // Check verifier_decision entry
    let no_verifier: Vec<_> = log_entries
        .iter()
        .filter(|e| {
            e.get("event_type").and_then(|v| v.as_str()) == Some("verifier_decision")
                && e.get("args")
                    .and_then(|a| a.get("verdict"))
                    .and_then(|v| v.as_str())
                    == Some("NoVerifier")
        })
        .collect();
    assert!(!no_verifier.is_empty(), "Expected NoVerifier entry");

    // Check human_approval entry with result=rejected and feedback
    let rejected_with_fb: Vec<_> = log_entries
        .iter()
        .filter(|e| {
            e.get("event_type").and_then(|v| v.as_str()) == Some("human_approval")
                && e.get("args")
                    .and_then(|a| a.get("result"))
                    .and_then(|v| v.as_str())
                    == Some("rejected")
                && e.get("args")
                    .and_then(|a| a.get("feedback"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.contains("unsafe"))
                    .unwrap_or(false)
        })
        .collect();
    assert!(
        !rejected_with_fb.is_empty(),
        "Expected rejected human_approval entry with feedback containing 'unsafe'"
    );

    // Verify the feedback text is correct
    let entry = &rejected_with_fb[0];
    let feedback = entry
        .get("args")
        .and_then(|a| a.get("feedback"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert_eq!(feedback, "This is unsafe, do not run");
}

// ---------------------------------------------------------------------------
// Tests: Verifier configured
// ---------------------------------------------------------------------------

#[test]
fn test_audit_verifier_allows() {
    let _lock = AUDIT_LOG_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _session = run_session(
        TestUi::always_yes(),
        Some("DECISION: ALLOW\nREASON: Safe Python command"),
    );

    let log_entries = read_audit_log();

    // Check verifier_decision entry with verdict=Allowed
    let allowed: Vec<_> = log_entries
        .iter()
        .filter(|e| {
            e.get("event_type").and_then(|v| v.as_str()) == Some("verifier_decision")
                && e.get("args")
                    .and_then(|a| a.get("verdict"))
                    .and_then(|v| v.as_str())
                    == Some("Allowed")
        })
        .collect();
    assert!(!allowed.is_empty(), "Expected Allowed verdict entry");

    // For auto-approved, there should be no human_approval entry
    let human_approvals: Vec<_> = log_entries
        .iter()
        .filter(|e| e.get("event_type").and_then(|v| v.as_str()) == Some("human_approval"))
        .collect();
    assert!(
        human_approvals.is_empty(),
        "Auto-approved should NOT create human_approval entries, got: {:?}",
        human_approvals
            .iter()
            .map(|e| e.get("event_type"))
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_audit_verifier_allows_reason_in_args() {
    let _lock = AUDIT_LOG_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _session = run_session(
        TestUi::always_yes(),
        Some("DECISION: ALLOW\nREASON: Intent matches tool call, no security risk"),
    );

    let log_entries = read_audit_log();

    // Check the reason is recorded
    let allowed: Vec<_> = log_entries
        .iter()
        .filter(|e| {
            e.get("event_type").and_then(|v| v.as_str()) == Some("verifier_decision")
                && e.get("args")
                    .and_then(|a| a.get("verdict"))
                    .and_then(|v| v.as_str())
                    == Some("Allowed")
        })
        .collect();
    assert!(!allowed.is_empty(), "Expected Allowed verdict entry");

    let reason = allowed[0]
        .get("args")
        .and_then(|a| a.get("reason"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    // verify_tool_call_full hardcodes reason to "Allowed" for Allowed verdicts
    // (VerificationResult::Allowed has no reason field).
    // The intent text (passed to verifier) is recorded elsewhere.
    assert_eq!(
        reason, "Allowed",
        "Allowed verdict reason should be 'Allowed'"
    );

    let auto_approved = allowed[0]
        .get("args")
        .and_then(|a| a.get("auto_approved"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    assert!(
        auto_approved,
        "Allowed verdict should have auto_approved=true"
    );
}

#[test]
fn test_audit_verifier_needs_approval_human_approves() {
    let _lock = AUDIT_LOG_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _session = run_session(
        TestUi::always_yes(),
        Some("DECISION: REVIEW\nREASON: File modification detected, requires human review"),
    );

    let log_entries = read_audit_log();

    // Check verifier_decision with NeedsApproval
    let needs_approval: Vec<_> = log_entries
        .iter()
        .filter(|e| {
            e.get("event_type").and_then(|v| v.as_str()) == Some("verifier_decision")
                && e.get("args")
                    .and_then(|a| a.get("verdict"))
                    .and_then(|v| v.as_str())
                    == Some("NeedsApproval")
        })
        .collect();
    assert!(
        !needs_approval.is_empty(),
        "Expected NeedsApproval verdict entry"
    );

    // Check human_approval with approved
    let approved: Vec<_> = log_entries
        .iter()
        .filter(|e| {
            e.get("event_type").and_then(|v| v.as_str()) == Some("human_approval")
                && e.get("args")
                    .and_then(|a| a.get("result"))
                    .and_then(|v| v.as_str())
                    == Some("approved")
        })
        .collect();
    assert!(
        !approved.is_empty(),
        "Expected approved human_approval entry"
    );

    // Check verifier_context
    let ctx = approved[0]
        .get("args")
        .and_then(|a| a.get("verifier_context"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert_eq!(ctx, "verifier_needs_approval");
}

#[test]
fn test_audit_verifier_needs_approval_human_rejects_with_feedback() {
    let _lock = AUDIT_LOG_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _session = run_session(
        TestUi::reject_with_feedback("I do not want to modify files"),
        Some("DECISION: REVIEW\nREASON: File modification detected"),
    );

    let log_entries = read_audit_log();

    // Check verifier_decision with NeedsApproval
    let needs_approval: Vec<_> = log_entries
        .iter()
        .filter(|e| {
            e.get("event_type").and_then(|v| v.as_str()) == Some("verifier_decision")
                && e.get("args")
                    .and_then(|a| a.get("verdict"))
                    .and_then(|v| v.as_str())
                    == Some("NeedsApproval")
        })
        .collect();
    assert!(
        !needs_approval.is_empty(),
        "Expected NeedsApproval verdict entry"
    );

    // Check human_approval with rejected and feedback
    let rejected: Vec<_> = log_entries
        .iter()
        .filter(|e| {
            e.get("event_type").and_then(|v| v.as_str()) == Some("human_approval")
                && e.get("args")
                    .and_then(|a| a.get("result"))
                    .and_then(|v| v.as_str())
                    == Some("rejected")
                && e.get("args")
                    .and_then(|a| a.get("feedback"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.contains("modify files"))
                    .unwrap_or(false)
        })
        .collect();
    assert!(
        !rejected.is_empty(),
        "Expected rejected human_approval with feedback"
    );

    let feedback = rejected[0]
        .get("args")
        .and_then(|a| a.get("feedback"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert_eq!(feedback, "I do not want to modify files");
}

#[test]
fn test_audit_verifier_fallback_human_approves() {
    let _lock = AUDIT_LOG_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _session = run_session(
        TestUi::always_yes(),
        Some("DECISION: BLOCK\nREASON: Verifier API unavailable"),
    );

    let log_entries = read_audit_log();

    // BLOCK maps to NeedsApproval in the current verifier logic
    // (BLOCK/REVIEW both map to NeedsApproval for human oversight)
    // So we check for NeedsApproval or FallbackRequired
    let flagged: Vec<_> = log_entries
        .iter()
        .filter(|e| {
            e.get("event_type").and_then(|v| v.as_str()) == Some("verifier_decision")
                && (e
                    .get("args")
                    .and_then(|a| a.get("verdict"))
                    .and_then(|v| v.as_str())
                    == Some("NeedsApproval")
                    || e.get("args")
                        .and_then(|a| a.get("verdict"))
                        .and_then(|v| v.as_str())
                        == Some("FallbackRequired"))
        })
        .collect();
    assert!(
        !flagged.is_empty(),
        "Expected NeedsApproval or FallbackRequired"
    );
}

// ---------------------------------------------------------------------------
// Tests: Audit log file integrity
// ---------------------------------------------------------------------------

#[test]
fn test_audit_log_file_contains_all_events() {
    let _lock = AUDIT_LOG_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _session = run_session(
        TestUi::reject_with_feedback("Not safe"),
        Some("DECISION: REVIEW\nREASON: Suspicious operation"),
    );

    // Read the on-disk audit log
    let log_entries = read_audit_log();

    // Should contain at least:
    // 1. session_start
    // 2. verifier_decision (NeedsApproval)
    // 3. human_approval (rejected with feedback)

    let event_types: Vec<&str> = log_entries
        .iter()
        .filter_map(|e| e.get("event_type").and_then(|v| v.as_str()))
        .collect();

    assert!(
        event_types.contains(&"session_start"),
        "Missing session_start: {:?}",
        event_types
    );
    assert!(
        event_types.contains(&"verifier_decision"),
        "Missing verifier_decision: {:?}",
        event_types
    );
    assert!(
        event_types.contains(&"human_approval"),
        "Missing human_approval: {:?}",
        event_types
    );

    // Check the args in the log file
    for entry in &log_entries {
        if entry.get("event_type").and_then(|v| v.as_str()) == Some("human_approval") {
            let result = entry
                .get("args")
                .and_then(|a| a.get("result"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            assert_eq!(result, "rejected", "human_approval should be rejected");
            let feedback = entry
                .get("args")
                .and_then(|a| a.get("feedback"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            assert_eq!(feedback, "Not safe", "feedback should match");
        }
    }
}

#[test]
fn test_audit_log_multiple_verifier_entries_have_unique_hashes() {
    let _lock = AUDIT_LOG_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _session = run_session(TestUi::always_yes(), Some("DECISION: ALLOW\nREASON: Safe"));

    let log_entries = read_audit_log();
    let verifier_entries: Vec<_> = log_entries
        .iter()
        .filter(|e| e.get("event_type").and_then(|v| v.as_str()) == Some("verifier_decision"))
        .collect();

    // Each entry should have a unique hash (Merkle chain integrity)
    let hashes: Vec<&str> = verifier_entries
        .iter()
        .filter_map(|e| e.get("hash").and_then(|v| v.as_str()))
        .collect();

    // All hashes should be non-empty
    for (i, h) in hashes.iter().enumerate() {
        assert_eq!(h.len(), 64, "Hash {} should be 64 hex chars", i);
    }

    // Hashes should be different from each other
    let unique_hashes: std::collections::HashSet<&str> = hashes.iter().cloned().collect();
    assert_eq!(
        unique_hashes.len(),
        hashes.len(),
        "All verifier_decision entries should have unique hashes"
    );
}
