//! Shared mock implementations for LLM-related tests.
//!
//! # Design
//!
//! This module provides reusable mock implementations of the core abstractions
//! that many integration tests depend on:
//!
//! * [`MockLlmClient`] — configurable mock LLM client for testing
//! * [`MockUi`] — silent UI stub (satisfies `UserInterface`)
//! * [`create_test_context`] — creates an [`AppContext`] with a temp base dir
//! * [`register_mock_client`] — registers a mock client factory in the registry
//!
//! # Usage
//!
//! ```ignore
//! use tests::common::*;
//!
//! let ctx = create_test_context(MockUi::confirming());
//! register_mock_client(&ctx, "mock_provider", Ok("Hello!".to_string())).await;
//! ```
//!
//! # Why not a framework (mockall, etc.)
//!
//! The [`LlmClient`] trait uses `async_trait`, which complicates mock
//! frameworks.  Manual mocks give us full control over async behaviour,
//! are easier to debug, and have zero compile-time overhead.

#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use serde_json::{Value, json};
use tempfile::TempDir;

use llm_secure_cli::cli::ui::{ConfirmResult, UserInterface};
use llm_secure_cli::core::context::AppContext;
use llm_secure_cli::llm::base::LlmClient;
use llm_secure_cli::llm::models::{
    ClientState, ContentPart, DataSource, LlmResponse, Message, MessagePart, Role,
};

// ---------------------------------------------------------------------------
// MockLlmClient — configurable mock for the LlmClient trait
// ---------------------------------------------------------------------------

/// Builds a [`MockLlmClient`] with a fluent API.
#[derive(Default)]
pub struct MockLlmClientBuilder {
    model: String,
    provider: String,
    tools_enabled: bool,
    responses: Vec<MockResponse>,
    verifier_response: Option<Result<Value, String>>,
    conversation: Vec<Message>,
    system_prompt: Option<String>,
    stdout: bool,
    raw: bool,
}

impl MockLlmClientBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn model(mut self, v: &str) -> Self {
        self.model = v.to_string();
        self
    }
    pub fn provider(mut self, v: &str) -> Self {
        self.provider = v.to_string();
        self
    }
    pub fn tools_enabled(mut self, v: bool) -> Self {
        self.tools_enabled = v;
        self
    }

    /// Add a sequential response for `send()`.  The client serves these in
    /// FIFO order; the last response is repeated once exhausted.
    #[expect(dead_code)]
    pub fn response(mut self, r: MockResponse) -> Self {
        self.responses.push(r);
        self
    }

    /// Shortcut: text-only response.
    pub fn text_response(mut self, text: &str) -> Self {
        self.responses.push(MockResponse::Text(text.to_string()));
        self
    }

    /// Shortcut: tool-call response.
    pub fn tool_call_response(mut self, name: &str, args: Value) -> Self {
        self.responses.push(MockResponse::ToolCall {
            name: name.to_string(),
            arguments: args,
        });
        self
    }

    /// Set the verifier response (defaults to `Allow`).
    pub fn verifier(mut self, r: Result<&str, &str>) -> Self {
        self.verifier_response = Some(
            r.map(|s| {
                json!({
                    "decision": "ALLOW",
                    "reason": s
                })
            })
            .map_err(|e| e.to_string()),
        );
        self
    }

    /// Pre-populate conversation history.
    pub fn history(mut self, msgs: Vec<Message>) -> Self {
        self.conversation = msgs;
        self
    }

    /// Place a system prompt (default: `None`).
    pub fn system_prompt(mut self, v: &str) -> Self {
        self.system_prompt = Some(v.to_string());
        self
    }

    pub fn build(self) -> MockLlmClient {
        MockLlmClient {
            state: ClientState {
                model: self.model,
                provider: self.provider,
                conversation: self.conversation,
                tools_enabled: self.tools_enabled,
                system_prompt_enabled: self.system_prompt.is_some(),
                system_prompt: self.system_prompt,
                stdout: self.stdout,
                render_markdown: !self.raw,
            },
            responses: self.responses,
            verifier_response: self
                .verifier_response
                .unwrap_or(Ok(json!({"decision": "ALLOW", "reason": "Mock allow"}))),
            call_count: AtomicUsize::new(0),
        }
    }
}

/// A single response for [`MockLlmClient::send()`].
#[derive(Clone, Debug)]
pub enum MockResponse {
    Text(String),
    ToolCall { name: String, arguments: Value },
    Error(String),
}

impl MockResponse {
    #[expect(dead_code)]
    pub fn text(s: &str) -> Self {
        Self::Text(s.to_string())
    }
    #[expect(dead_code)]
    pub fn tool_call(name: &str, args: Value) -> Self {
        Self::ToolCall {
            name: name.to_string(),
            arguments: args,
        }
    }
    pub fn error(s: &str) -> Self {
        Self::Error(s.to_string())
    }
}

/// Flexible mock LLM client for testing.
///
/// Serves responses in FIFO order for `send()`, and returns a fixed
/// `verifier_response` for `send_as_verifier()`.
pub struct MockLlmClient {
    pub state: ClientState,
    pub responses: Vec<MockResponse>,
    pub verifier_response: Result<Value, String>,
    pub call_count: AtomicUsize,
}

impl MockLlmClient {
    /// Create a builder.
    pub fn builder() -> MockLlmClientBuilder {
        MockLlmClientBuilder::new()
    }

    /// Build an empty client (returns empty string for every `send()` call).
    #[expect(dead_code)]
    pub fn empty() -> Self {
        Self::builder().text_response("").build()
    }

    fn next_response(&self) -> MockResponse {
        let count = self.call_count.fetch_add(1, Ordering::SeqCst);
        self.responses
            .get(count)
            .or_else(|| self.responses.last())
            .cloned()
            .unwrap_or(MockResponse::Text(String::new()))
    }

    fn make_assistant_message(response: &MockResponse) -> (Message, LlmResponse) {
        match response {
            MockResponse::Text(text) => {
                let msg = Message {
                    role: Role::Assistant,
                    parts: vec![MessagePart::Part(Box::new(ContentPart {
                        text: Some(text.clone()),
                        is_diagnostic: false,
                        ..Default::default()
                    }))],
                };
                let llm_resp = LlmResponse {
                    content: Some(text.clone()),
                    tool_name: None,
                    usage: None,
                };
                (msg, llm_resp)
            }
            MockResponse::ToolCall { name, arguments } => {
                let mut fc = std::collections::HashMap::new();
                fc.insert("name".to_string(), json!(name));
                fc.insert("arguments".to_string(), arguments.clone());
                fc.insert("id".to_string(), json!("mock_call_id"));
                let msg = Message {
                    role: Role::Assistant,
                    parts: vec![MessagePart::Part(Box::new(ContentPart {
                        function_call: Some(fc),
                        is_diagnostic: false,
                        ..Default::default()
                    }))],
                };
                let llm_resp = LlmResponse {
                    content: None,
                    tool_name: Some(name.clone()),
                    usage: None,
                };
                (msg, llm_resp)
            }
            MockResponse::Error(_err) => {
                let msg = Message {
                    role: Role::Assistant,
                    parts: vec![],
                };
                let llm_resp = LlmResponse {
                    content: None,
                    tool_name: None,
                    usage: None,
                };
                (msg, llm_resp)
            }
        }
    }
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
        &self.state.provider
    }
    fn should_send_pdf_as_base64(&self) -> bool {
        false
    }

    async fn send(
        &mut self,
        data: Vec<DataSource>,
        _tool_schemas: Vec<Value>,
    ) -> anyhow::Result<LlmResponse> {
        let response = self.next_response();
        match &response {
            MockResponse::Error(err) => Err(anyhow::anyhow!(err.clone())),
            _ => {
                let (msg, resp) = Self::make_assistant_message(&response);
                self.update_history(&data, msg);
                Ok(resp)
            }
        }
    }

    async fn send_as_verifier(
        &mut self,
        _data: Vec<DataSource>,
        _tool_schema: Value,
    ) -> anyhow::Result<Value> {
        match &self.verifier_response {
            Ok(v) => Ok(v.clone()),
            Err(e) => Err(anyhow::anyhow!(e.clone())),
        }
    }
}

// ---------------------------------------------------------------------------
// MockUi — silent UI stub
// ---------------------------------------------------------------------------

/// A [`UserInterface`] implementation that records calls for assertions.
///
/// By default all methods are silent no-ops.  Use the builder to attach
/// behaviour to specific methods.
#[derive(Clone, Default)]
pub struct MockUi {
    pub confirmed: Option<bool>,
    pub messages: Arc<Mutex<Vec<String>>>,
    /// If `Some(...)`, `ask_confirm` returns `Feedback(value)` instead of `Yes`/`No`.
    pub feedback_text: Option<String>,
}

impl MockUi {
    /// Returns a UI that always confirms (returns `Yes`).
    pub fn confirming() -> Self {
        Self {
            confirmed: Some(true),
            ..Default::default()
        }
    }

    /// Returns a UI that always rejects (returns `No`).
    pub fn rejecting() -> Self {
        Self {
            confirmed: Some(false),
            feedback_text: None,
            ..Default::default()
        }
    }

    /// Returns a UI that rejects with the given feedback text.
    #[allow(dead_code)]
    pub fn rejecting_with_feedback(feedback: &str) -> Self {
        Self {
            confirmed: Some(false),
            feedback_text: Some(feedback.to_string()),
            ..Default::default()
        }
    }

    /// Collect all reported messages for assertion.
    pub fn collected_messages(&self) -> Vec<String> {
        self.messages.lock().unwrap().clone()
    }

    fn record(&self, msg: String) {
        if let Ok(mut msgs) = self.messages.lock() {
            msgs.push(msg);
        }
    }
}

#[async_trait]
impl UserInterface for MockUi {
    fn print_block(&self, _c: &str, _t: Option<&str>, _s: Option<&str>) {}
    fn print_rule(&self, _t: Option<&str>, _s: Option<&str>) {}
    fn print_tool_call(&self, _n: &str, _a: &Value) {}
    fn print_tool_call_direct(&self, _n: &str, _a: &Value) {}
    fn print_tool_result(&self, _r: &str) {}
    fn report_error(&self, m: &str) {
        self.record(format!("ERROR: {}", m));
    }
    fn report_info(&self, m: &str) {
        self.record(format!("INFO: {}", m));
    }
    fn report_warning(&self, m: &str) {
        self.record(format!("WARN: {}", m));
    }
    fn report_success(&self, m: &str) {
        self.record(format!("SUCCESS: {}", m));
    }

    async fn ask_confirm(&self, _p: &str) -> Option<ConfirmResult> {
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
    async fn ask_confirm_simple(&self, _p: &str) -> Option<ConfirmResult> {
        self.confirmed.map(|y| {
            if y {
                ConfirmResult::Yes
            } else {
                ConfirmResult::No
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Context creation helpers
// ---------------------------------------------------------------------------

use std::sync::LazyLock;

static TEST_TEMP_DIR: LazyLock<Arc<Mutex<Option<TempDir>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(None)));

/// Initialize the test environment once (process-wide but safe).
///
/// Sets a temporary base directory so tests don't read/write `~/.llm_secure_cli`.
pub fn init_test_env() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let tmp = TempDir::new().expect("Failed to create temp dir");
        let path = tmp.path().to_path_buf();
        *TEST_TEMP_DIR.lock().unwrap() = Some(tmp);
        llm_secure_cli::consts::init_base_dir(Some(path));
    });
}

/// Creates an [`AppContext`] backed by a temporary directory and the given UI.
pub fn create_test_context(ui: MockUi) -> Arc<AppContext> {
    init_test_env();
    Arc::new(AppContext::new(Arc::new(ui)))
}

/// Registers a [`MockLlmClient`] factory in the context's client registry.
///
/// The factory creates clients that return `response_text` for every
/// `send()` call.
pub async fn register_mock_client(
    ctx: &Arc<AppContext>,
    provider_name: &str,
    response: Result<String, String>,
) {
    let mut registry = ctx.client_registry.lock().await;
    let provider = provider_name.to_string();
    let provider_clone = provider.clone();
    registry.register(
        &provider,
        Arc::new(move |_model, _stdout, _raw, _cfg| {
            let r = response.clone();
            let p = provider_clone.clone();
            let mut builder = MockLlmClient::builder().provider(&p).model("mock-model");
            match &r {
                Ok(t) => {
                    builder = builder.text_response(t);
                }
                Err(e) => {
                    builder = MockLlmClientBuilder {
                        responses: vec![MockResponse::Error(e.clone())],
                        verifier_response: Some(Err(e.clone())),
                        ..builder
                    };
                }
            }
            Ok(Box::new(builder.build()) as Box<dyn LlmClient>)
        }),
    );
}

/// Registers a flexible mock client that returns a pre-built `MockLlmClient`.
#[expect(dead_code)]
pub async fn register_mock_full(ctx: &Arc<AppContext>, provider_name: &str, mock: MockLlmClient) {
    let mut registry = ctx.client_registry.lock().await;
    let provider = provider_name.to_string();
    let provider_clone = provider.clone();
    registry.register(
        &provider,
        Arc::new(move |_model, _stdout, _raw, _cfg| {
            let mut new_mock = MockLlmClient::builder()
                .provider(&provider_clone)
                .model(&mock.state.model)
                .tools_enabled(mock.state.tools_enabled);
            for r in &mock.responses {
                new_mock = match r {
                    MockResponse::Text(t) => new_mock.text_response(t),
                    MockResponse::ToolCall { name, arguments } => {
                        new_mock.tool_call_response(name, arguments.clone())
                    }
                    MockResponse::Error(e) => MockLlmClientBuilder {
                        responses: vec![MockResponse::Error(e.clone())],
                        ..new_mock
                    },
                };
            }
            Ok(Box::new(new_mock.build()) as Box<dyn LlmClient>)
        }),
    );
}
