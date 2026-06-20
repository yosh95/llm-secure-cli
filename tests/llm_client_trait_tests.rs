//! Integration tests for the [`LlmClient`] trait using [`MockLlmClient`].
//!
//! These tests verify that:
//! - The trait's default method implementations work correctly
//! - Session save/load round-trips correctly
//! - Conversation state management behaves as expected
//! - The mock client itself works correctly

#![allow(clippy::unwrap_used, clippy::expect_used)]
mod common;

use common::*;
use llm_secure_cli::cli::ui::UserInterface;
use llm_secure_cli::llm::base::LlmClient;
use llm_secure_cli::llm::models::{ClientState, DataSource, Message, MessagePart, Role};
use serde_json::{Value, json};

// ===========================================================================
// MockLlmClient construction & basic behaviour
// ===========================================================================

#[test]
fn test_mock_client_builder_defaults() {
    let client = MockLlmClient::builder().build();
    assert_eq!(client.state.model, "");
    assert_eq!(client.state.provider, "");
    assert!(!client.state.tools_enabled);
    assert!(client.state.system_prompt.is_none());
    assert!(!client.state.stdout);
    assert!(client.state.render_markdown);
}

#[test]
fn test_mock_client_builder_full_config() {
    let client = MockLlmClient::builder()
        .model("gpt-4")
        .provider("openai")
        .tools_enabled(true)
        .system_prompt("You are helpful.")
        .text_response("Hello!")
        .build();
    assert_eq!(client.state.model, "gpt-4");
    assert_eq!(client.state.provider, "openai");
    assert!(client.state.tools_enabled);
    assert_eq!(
        client.state.system_prompt.as_deref(),
        Some("You are helpful.")
    );
    assert_eq!(client.responses.len(), 1);
}

// ===========================================================================
// get_state / get_state_mut
// ===========================================================================

#[test]
fn test_get_state_reflects_initial_state() {
    let client = MockLlmClient::builder()
        .model("claude-3")
        .provider("anthropic")
        .build();
    let state: &ClientState = LlmClient::get_state(&client);
    assert_eq!(state.model, "claude-3");
    assert_eq!(state.provider, "anthropic");
}

#[test]
fn test_get_state_mut_allows_modification() {
    let mut client = MockLlmClient::builder().build();
    let state = LlmClient::get_state_mut(&mut client);
    state.model = "modified".to_string();
    assert_eq!(client.state.model, "modified");
}

// ===========================================================================
// get_display_name
// ===========================================================================

#[test]
fn test_display_name_known_providers() {
    let client = MockLlmClient::builder()
        .provider("openai")
        .model("gpt-4")
        .build();
    assert_eq!(client.get_display_name(), "OpenAI (gpt-4)");

    let client = MockLlmClient::builder()
        .provider("ollama")
        .model("llama3")
        .build();
    assert_eq!(client.get_display_name(), "Ollama (llama3)");

    let client = MockLlmClient::builder()
        .provider("openrouter")
        .model("anthropic/claude-3")
        .build();
    assert_eq!(client.get_display_name(), "OpenRouter (anthropic/claude-3)");
}

#[test]
fn test_display_name_unknown_provider() {
    let client = MockLlmClient::builder()
        .provider("custom")
        .model("my-model")
        .build();
    assert_eq!(client.get_display_name(), "Custom (my-model)");
}

#[test]
fn test_display_name_empty_provider() {
    let client = MockLlmClient::builder().provider("").model("model").build();
    assert_eq!(client.get_display_name(), "LLM (model)");
}

// ===========================================================================
// get_config_section
// ===========================================================================

#[test]
fn test_get_config_section_returns_provider() {
    let client = MockLlmClient::builder().provider("my-provider").build();
    assert_eq!(client.get_config_section(), "my-provider");
}

// ===========================================================================
// should_send_pdf_as_base64
// ===========================================================================

#[test]
fn test_mock_returns_false_for_pdf() {
    let client = MockLlmClient::builder().build();
    assert!(!client.should_send_pdf_as_base64());
}

// ===========================================================================
// send() with mock responses
// ===========================================================================

#[test]
fn test_send_text_response() {
    let mut client = MockLlmClient::builder()
        .text_response("Hello, this is a test!")
        .build();
    let response = client.send(vec![], vec![]).expect("send should succeed");
    assert_eq!(response.content.as_deref(), Some("Hello, this is a test!"));
    assert!(response.tool_name.is_none());
}

#[test]
fn test_send_tool_call_response() {
    let mut client = MockLlmClient::builder()
        .tool_call_response("list_files", json!({"path": "."}))
        .build();
    let response = client.send(vec![], vec![]).expect("send should succeed");
    assert!(
        response.content.is_none(),
        "Tool call responses have no content"
    );
    assert_eq!(response.tool_name.as_deref(), Some("list_files"));
}

#[test]
fn test_send_error_response() {
    let mut client = MockLlmClient {
        responses: vec![MockResponse::error("API timeout")],
        ..MockLlmClient::builder().build()
    };
    let result = client.send(vec![], vec![]);
    assert!(result.is_err(), "Error responses should return Err");
    assert!(result.unwrap_err().to_string().contains("API timeout"));
}

#[test]
fn test_send_multiple_responses_fifo() {
    let mut client = MockLlmClient::builder()
        .text_response("First")
        .text_response("Second")
        .text_response("Third")
        .build();
    let r1 = client.send(vec![], vec![]).unwrap();
    assert_eq!(r1.content.as_deref(), Some("First"));
    let r2 = client.send(vec![], vec![]).unwrap();
    assert_eq!(r2.content.as_deref(), Some("Second"));
    let r3 = client.send(vec![], vec![]).unwrap();
    assert_eq!(r3.content.as_deref(), Some("Third"));
}

#[test]
fn test_send_exhausted_repeats_last_response() {
    let mut client = MockLlmClient::builder().text_response("Only one").build();
    let _ = client.send(vec![], vec![]).unwrap();
    let r2 = client.send(vec![], vec![]).unwrap();
    assert_eq!(
        r2.content.as_deref(),
        Some("Only one"),
        "Last response should repeat once exhausted"
    );
}

// ===========================================================================
// update_history — conversation management
// ===========================================================================

#[test]
fn test_update_history_adds_user_and_assistant_messages() {
    let mut client = MockLlmClient::builder()
        .text_response("Sure, I can help!")
        .build();

    let data = vec![DataSource {
        content: Value::String("What is Rust?".to_string()),
        content_type: "text/plain".to_string(),
        is_file_or_url: false,
        metadata: Default::default(),
    }];

    // send() internally calls update_history
    let _ = client.send(data.clone(), vec![]).unwrap();

    assert_eq!(client.state.conversation.len(), 2);
    assert_eq!(client.state.conversation[0].role, Role::User);
    assert_eq!(client.state.conversation[1].role, Role::Assistant);

    // User message content
    let user_text = client.state.conversation[0].get_text(false);
    assert_eq!(user_text, "What is Rust?");
}

#[test]
fn test_update_history_consecutive_calls_accumulate() {
    let mut client = MockLlmClient::builder()
        .text_response("First response")
        .text_response("Second response")
        .build();

    let data1 = vec![DataSource {
        content: Value::String("Msg 1".to_string()),
        content_type: "text/plain".to_string(),
        is_file_or_url: false,
        metadata: Default::default(),
    }];
    let data2 = vec![DataSource {
        content: Value::String("Msg 2".to_string()),
        content_type: "text/plain".to_string(),
        is_file_or_url: false,
        metadata: Default::default(),
    }];

    let _ = client.send(data1, vec![]).unwrap();
    let _ = client.send(data2, vec![]).unwrap();

    assert_eq!(client.state.conversation.len(), 4);
    assert_eq!(client.state.conversation[0].role, Role::User);
    assert_eq!(client.state.conversation[1].role, Role::Assistant);
    assert_eq!(client.state.conversation[2].role, Role::User);
    assert_eq!(client.state.conversation[3].role, Role::Assistant);
}

// ===========================================================================
// send_as_verifier
// ===========================================================================

#[test]
fn test_send_as_verifier_returns_configured_response() {
    let mut client = MockLlmClient::builder()
        .verifier(Ok("Safe request"))
        .build();
    let result = client.send_as_verifier(vec![], json!({}));
    assert!(result.is_ok());
    let val = result.unwrap();
    assert_eq!(val["decision"], "ALLOW");
    assert_eq!(val["reason"], "Safe request");
}

#[test]
fn test_send_as_verifier_returns_error() {
    let mut client = MockLlmClient::builder()
        .verifier(Err("API unavailable"))
        .build();
    let result = client.send_as_verifier(vec![], json!({}));
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("API unavailable"));
}

// ===========================================================================
// save_session / load_session round-trip
// ===========================================================================

#[test]
fn test_session_save_and_load_round_trip() {
    use tempfile::TempDir;

    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("session.json");

    let client_a = MockLlmClient::builder()
        .history(vec![
            Message {
                role: Role::User,
                parts: vec![MessagePart::Text("Hello".to_string())],
            },
            Message {
                role: Role::Assistant,
                parts: vec![MessagePart::Text("Hi!".to_string())],
            },
        ])
        .build();

    // Save
    client_a
        .save_session(path.to_str().unwrap())
        .expect("save should succeed");

    // Load into a fresh client
    let mut client_b = MockLlmClient::builder().build();
    client_b
        .load_session(path.to_str().unwrap())
        .expect("load should succeed");

    assert_eq!(client_b.state.conversation.len(), 2);
    assert_eq!(client_b.state.conversation[0].role, Role::User);
    assert_eq!(client_b.state.conversation[0].get_text(false), "Hello");
    assert_eq!(client_b.state.conversation[1].role, Role::Assistant);
    assert_eq!(client_b.state.conversation[1].get_text(false), "Hi!");
}

#[test]
fn test_save_session_empty_conversation() {
    use tempfile::TempDir;
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("empty.json");

    let client = MockLlmClient::builder().build();
    client
        .save_session(path.to_str().unwrap())
        .expect("save empty should succeed");

    let content = std::fs::read_to_string(&path).unwrap();
    assert_eq!(content, "[]");
}

// ===========================================================================
// get_effective_system_prompt
// ===========================================================================

#[test]
fn test_system_prompt_disabled_returns_none() {
    let mut client = MockLlmClient::builder().build();
    client.state.system_prompt_enabled = false;
    assert!(client.state.get_effective_system_prompt().is_none());
}

#[test]
fn test_system_prompt_enabled_without_custom_prompt() {
    let client = MockLlmClient::builder().system_prompt("").build();
    let prompt = client.state.get_effective_system_prompt();
    assert!(prompt.is_some());
    let text = prompt.unwrap();
    assert!(text.contains("Today's date is"));
}

#[test]
fn test_system_prompt_enabled_with_custom_prompt() {
    let client = MockLlmClient::builder()
        .system_prompt("You are an AI assistant specialized in Rust.")
        .build();
    let prompt = client.state.get_effective_system_prompt();
    assert!(prompt.is_some());
    let text = prompt.unwrap();
    assert!(text.contains("Rust"));
    assert!(text.contains("Today's date is"));
}

// ===========================================================================
// Conversation manipulation
// ===========================================================================

#[test]
fn test_conversation_can_be_cleared_and_replaced() {
    let mut client = MockLlmClient::builder()
        .history(vec![Message {
            role: Role::System,
            parts: vec![MessagePart::Text("Initial".to_string())],
        }])
        .build();
    assert_eq!(client.state.conversation.len(), 1);

    // Replace conversation
    client.get_state_mut().conversation = vec![Message {
        role: Role::User,
        parts: vec![MessagePart::Text("New start".to_string())],
    }];
    assert_eq!(client.state.conversation.len(), 1);
    assert_eq!(client.state.conversation[0].role, Role::User);
}

// ===========================================================================
// MockUi tests
// ===========================================================================

#[test]
fn test_mock_ui_confirming() {
    let ui = MockUi::confirming();
    assert_eq!(ui.confirmed, Some(true));
}

#[test]
fn test_mock_ui_rejecting() {
    let ui = MockUi::rejecting();
    assert_eq!(ui.confirmed, Some(false));
}

#[test]
fn test_mock_ui_ask_confirm() {
    let ui = MockUi::confirming();
    let result = ui.ask_confirm("Proceed?");
    assert_eq!(result, Some(llm_secure_cli::cli::ui::ConfirmResult::Yes));
}

#[test]
fn test_mock_ui_records_messages() {
    let ui = MockUi::default();
    ui.report_info("Info message");
    ui.report_warning("Warning message");
    ui.report_error("Error message");
    ui.report_success("Success message");

    let msgs = ui.collected_messages();
    assert_eq!(msgs.len(), 4);
    assert!(msgs[0].contains("INFO:"));
    assert!(msgs[1].contains("WARN:"));
    assert!(msgs[2].contains("ERROR:"));
    assert!(msgs[3].contains("SUCCESS:"));
}

// ===========================================================================
// create_test_context integration
// ===========================================================================

#[test]
fn test_create_test_context_uses_temp_dir() {
    let ctx = create_test_context(MockUi::confirming());
    let config = ctx.config_manager.get_config();
    assert!(
        config.is_ok(),
        "Config should load successfully from temp dir"
    );
}

#[test]
fn test_register_mock_client_and_use_it() {
    let ctx = create_test_context(MockUi::confirming());
    register_mock_client(&ctx, "test_provider", Ok("Mock response".to_string()));

    let registry = ctx.client_registry.lock().unwrap();
    let mut client = registry
        .create_client("test_provider", "model", false, true, &ctx.config_manager)
        .expect("Should create client");

    let response = client.send(vec![], vec![]).expect("send should succeed");
    assert_eq!(response.content.as_deref(), Some("Mock response"));
}

#[test]
fn test_register_mock_client_error() {
    let ctx = create_test_context(MockUi::confirming());
    register_mock_client(&ctx, "bad_provider", Err("Network error".to_string()));

    let registry = ctx.client_registry.lock().unwrap();
    let mut client = registry
        .create_client("bad_provider", "model", false, true, &ctx.config_manager)
        .expect("Should create client");

    let result = client.send(vec![], vec![]);
    assert!(result.is_err(), "Error mock should return error");
}
