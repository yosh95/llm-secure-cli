//! MessageBuilder tests — pure-function unit tests for the
//! OpenAI-compatible message array construction.
//!
//! These tests verify that the internal conversation model is correctly
//! serialized into the wire format without an HTTP round-trip.
//! The `MessageBuilder` is extracted as a standalone struct precisely
//! for this kind of testing.

#![allow(clippy::unwrap_used, clippy::expect_used)]
use llm_secure_cli::llm::models::{ContentPart, DataSource, Message, MessagePart, Role};
use llm_secure_cli::llm::providers::message_builder::MessageBuilder;
use llm_secure_cli::llm::providers::payload_formatter::GenericPayloadFormatter;
use serde_json::Value;

/// Helper: builds a MessageBuilder with minimal boilerplate.
fn builder(
    system_prompt: Option<&str>,
    conversation: Vec<Message>,
    pending_data: Vec<DataSource>,
) -> MessageBuilder<'static> {
    // The GenericPayloadFormatter is a singleton with no state; a leak is fine.
    let formatter: &'static dyn llm_secure_cli::llm::providers::payload_formatter::PayloadFormatter =
        Box::leak(Box::new(GenericPayloadFormatter));
    MessageBuilder {
        formatter,
        model: "test-model",
        input_modalities: None,
        system_prompt: system_prompt.map(|s| s.to_string()),
        conversation: Box::leak(Box::new(conversation)),
        pending_data: Box::leak(Box::new(pending_data)),
    }
}

fn simple_user(text: &str) -> Message {
    Message {
        role: Role::User,
        parts: vec![MessagePart::Text(text.to_string())],
    }
}

fn simple_assistant(text: &str) -> Message {
    Message {
        role: Role::Assistant,
        parts: vec![MessagePart::Part(Box::new(ContentPart {
            text: Some(text.to_string()),
            is_diagnostic: false,
            ..Default::default()
        }))],
    }
}

// ===========================================================================
// System prompt
// ===========================================================================

#[test]
fn test_empty_conversation_produces_system_prompt_only() {
    let msgs = builder(Some("You are a helpful AI."), vec![], vec![]).build();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["role"], "system");
    assert_eq!(msgs[0]["content"], "You are a helpful AI.");
}

#[test]
fn test_no_system_prompt_when_none_is_set() {
    let msgs = builder(None, vec![], vec![]).build();
    assert_eq!(msgs.len(), 0, "No system prompt → no messages");
}

#[test]
fn test_system_prompt_with_date_directive_included() {
    // When constructing a MessageBuilder, the system_prompt field
    // already contains the final text (with date directive).
    // We test that the text is passed through correctly.
    let prompt = "You are a helpful AI.\n\nToday's date is 2026-05-24. You must treat this as...";
    let msgs = builder(Some(prompt), vec![], vec![]).build();
    assert!(
        msgs[0]["content"]
            .as_str()
            .unwrap()
            .contains("Today's date is")
    );
}

// ===========================================================================
// Basic user/assistant turns
// ===========================================================================

#[test]
fn test_single_user_message() {
    let msgs = builder(Some("System"), vec![simple_user("Hello!")], vec![]).build();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0]["role"], "system");
    assert_eq!(msgs[1]["role"], "user");
    assert_eq!(msgs[1]["content"], "Hello!");
}

#[test]
fn test_simple_conversation_round_trip() {
    let msgs = builder(
        Some("System"),
        vec![
            simple_user("Hello"),
            simple_assistant("Hi there!"),
            simple_user("What's the weather?"),
        ],
        vec![],
    )
    .build();
    assert_eq!(msgs.len(), 4);
    assert_eq!(msgs[0]["role"], "system");
    assert_eq!(msgs[1]["role"], "user");
    assert_eq!(msgs[2]["role"], "assistant");
    assert_eq!(msgs[3]["role"], "user");
    assert_eq!(msgs[1]["content"], "Hello");
    assert_eq!(msgs[2]["content"], "Hi there!");
    assert_eq!(msgs[3]["content"], "What's the weather?");
}

// ===========================================================================
// Pending data (current user input)
// ===========================================================================

#[test]
fn test_pending_data_appended_as_user_message() {
    let pending = vec![DataSource {
        content: Value::String("What is Rust?".to_string()),
        content_type: "text/plain".to_string(),
        is_file_or_url: false,
        metadata: Default::default(),
    }];
    let msgs = builder(Some("System"), vec![], pending).build();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[1]["role"], "user");
    assert_eq!(msgs[1]["content"], "What is Rust?");
}

#[test]
fn test_pending_data_combined_with_existing_messages() {
    let pending = vec![DataSource {
        content: Value::String("Now translate it.".to_string()),
        content_type: "text/plain".to_string(),
        is_file_or_url: false,
        metadata: Default::default(),
    }];
    let msgs = builder(
        Some("System"),
        vec![simple_user("Hello"), simple_assistant("Hi!")],
        pending,
    )
    .build();
    assert_eq!(msgs.len(), 4);
    assert_eq!(msgs[3]["role"], "user");
    assert_eq!(msgs[3]["content"], "Now translate it.");
}

// ===========================================================================
// Tool calls
// ===========================================================================

#[test]
fn test_tool_call_in_assistant_message_has_tool_calls_field() {
    use std::collections::HashMap;
    let mut fc = HashMap::new();
    fc.insert("name".to_string(), Value::String("list_files".to_string()));
    fc.insert(
        "arguments".to_string(),
        Value::String(r#"{"path": "."}"#.to_string()),
    );
    fc.insert("id".to_string(), Value::String("call_1".to_string()));

    let msg = Message {
        role: Role::Assistant,
        parts: vec![MessagePart::Part(Box::new(ContentPart {
            function_call: Some(fc),
            ..Default::default()
        }))],
    };

    let msgs = builder(Some("System"), vec![msg], vec![]).build();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[1]["role"], "assistant");
    let tool_calls = msgs[1]["tool_calls"]
        .as_array()
        .expect("Should have tool_calls");
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0]["function"]["name"], "list_files");
    assert_eq!(tool_calls[0]["type"], "function");
    assert_eq!(tool_calls[0]["id"], "call_1");
}

#[test]
fn test_tool_response_uses_tool_call_id_when_available() {
    use std::collections::HashMap;
    // First, assistant issues a tool call
    let mut fc = HashMap::new();
    fc.insert("name".to_string(), Value::String("list_files".to_string()));
    fc.insert(
        "arguments".to_string(),
        Value::String(r#"{"path": "."}"#.to_string()),
    );
    fc.insert("id".to_string(), Value::String("call_1".to_string()));

    let assistant_msg = Message {
        role: Role::Assistant,
        parts: vec![MessagePart::Part(Box::new(ContentPart {
            function_call: Some(fc),
            ..Default::default()
        }))],
    };

    // Then, tool response
    let mut fr = HashMap::new();
    fr.insert("id".to_string(), Value::String("call_1".to_string()));
    fr.insert("name".to_string(), Value::String("list_files".to_string()));
    fr.insert(
        "response".to_string(),
        Value::String("file1, file2".to_string()),
    );

    let tool_msg = Message {
        role: Role::Tool,
        parts: vec![MessagePart::Part(Box::new(ContentPart {
            function_response: Some(fr),
            ..Default::default()
        }))],
    };

    let msgs = builder(Some("System"), vec![assistant_msg, tool_msg], vec![]).build();
    assert_eq!(msgs.len(), 3); // system + assistant + tool
    assert_eq!(msgs[1]["role"], "assistant");
    assert_eq!(msgs[2]["role"], "tool");
    assert_eq!(msgs[2]["tool_call_id"], "call_1");
    assert_eq!(msgs[2]["content"], "file1, file2");
}

#[test]
fn test_tool_response_without_id_falls_back_to_user_message() {
    // If the tool response has no matching id, it's sent as a user message
    let mut fr = std::collections::HashMap::new();
    fr.insert("name".to_string(), Value::String("list_files".to_string()));
    fr.insert("response".to_string(), Value::String("file1".to_string()));
    // No "id" field

    let tool_msg = Message {
        role: Role::Tool,
        parts: vec![MessagePart::Part(Box::new(ContentPart {
            function_response: Some(fr),
            ..Default::default()
        }))],
    };

    let msgs = builder(Some("System"), vec![tool_msg], vec![]).build();
    // The last message should have role: "user" (fallback)
    let last = msgs.last().unwrap();
    assert_eq!(
        last["role"], "user",
        "Orphaned tool responses should become user messages"
    );
}

// ===========================================================================
// Multi-part content (text + images)
// ===========================================================================

#[test]
fn test_multi_part_user_message_with_image() {
    use std::collections::HashMap;
    let mut id = HashMap::new();
    id.insert(
        "mimeType".to_string(),
        Value::String("image/png".to_string()),
    );
    id.insert("data".to_string(), Value::String("base64data".to_string()));

    let msg = Message {
        role: Role::User,
        parts: vec![
            MessagePart::Text("What's in this image?".to_string()),
            MessagePart::Part(Box::new(ContentPart {
                inline_data: Some(id),
                ..Default::default()
            })),
        ],
    };

    let msgs = builder(None, vec![msg], vec![]).build();
    assert_eq!(msgs.len(), 1);
    let content = msgs[0]["content"]
        .as_array()
        .expect("Multi-part content should be an array");
    assert_eq!(content.len(), 2);

    // First part: text
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[0]["text"], "What's in this image?");

    // Second part: image_url
    assert_eq!(content[1]["type"], "image_url");
    let url = content[1]["image_url"]["url"].as_str().unwrap();
    assert!(url.starts_with("data:image/png;base64,"));
}

// ===========================================================================
// Consecutive assistant messages (edge case)
// ===========================================================================

#[test]
fn test_consecutive_assistant_messages() {
    let msgs = builder(
        None,
        vec![simple_assistant("First"), simple_assistant("Second")],
        vec![],
    )
    .build();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0]["role"], "assistant");
    assert_eq!(msgs[0]["content"], "First");
    assert_eq!(msgs[1]["role"], "assistant");
    assert_eq!(msgs[1]["content"], "Second");
}

// ===========================================================================
// Role::Model treated as assistant
// ===========================================================================

#[test]
fn test_model_role_treated_as_assistant() {
    let msg = Message {
        role: Role::Model,
        parts: vec![MessagePart::Text("Model output".to_string())],
    };
    let msgs = builder(None, vec![msg], vec![]).build();
    assert_eq!(msgs[0]["role"], "assistant");
    assert_eq!(msgs[0]["content"], "Model output");
}

// ===========================================================================
// Empty parts in messages
// ===========================================================================

#[test]
fn test_message_with_no_parts_produces_empty_content() {
    let msg = Message {
        role: Role::User,
        parts: vec![],
    };
    let msgs = builder(None, vec![msg], vec![]).build();
    assert_eq!(msgs[0]["content"], "");
}

// ===========================================================================
// Tool call count tracking
// ===========================================================================

#[test]
fn test_multiple_tool_calls_generate_multiple_tool_calls_array_entries() {
    use std::collections::HashMap;

    let mut fc1 = HashMap::new();
    fc1.insert("name".to_string(), Value::String("tool_a".to_string()));
    fc1.insert(
        "arguments".to_string(),
        Value::String(r#"{"a":1}"#.to_string()),
    );
    fc1.insert("id".to_string(), Value::String("call_a".to_string()));

    let mut fc2 = HashMap::new();
    fc2.insert("name".to_string(), Value::String("tool_b".to_string()));
    fc2.insert(
        "arguments".to_string(),
        Value::String(r#"{"b":2}"#.to_string()),
    );
    fc2.insert("id".to_string(), Value::String("call_b".to_string()));

    // Note: Multiple function_calls in a single message are represented as
    // multiple MessagePart entries. The builder should group them.
    let msg = Message {
        role: Role::Assistant,
        parts: vec![
            MessagePart::Part(Box::new(ContentPart {
                function_call: Some(fc1),
                ..Default::default()
            })),
            MessagePart::Part(Box::new(ContentPart {
                function_call: Some(fc2),
                ..Default::default()
            })),
        ],
    };

    let msgs = builder(None, vec![msg], vec![]).build();
    let tool_calls = msgs[0]["tool_calls"]
        .as_array()
        .expect("Should have tool_calls");
    assert_eq!(tool_calls.len(), 2);
    assert_eq!(tool_calls[0]["function"]["name"], "tool_a");
    assert_eq!(tool_calls[0]["id"], "call_a");
    assert_eq!(tool_calls[1]["function"]["name"], "tool_b");
    assert_eq!(tool_calls[1]["id"], "call_b");
}

// ===========================================================================
// Input modality filtering
// ===========================================================================

#[test]
fn test_image_included_when_modality_supported() {
    use std::collections::HashMap;
    let mut id = HashMap::new();
    id.insert(
        "mimeType".to_string(),
        Value::String("image/png".to_string()),
    );
    id.insert("data".to_string(), Value::String("base64data".to_string()));

    let msg = Message {
        role: Role::User,
        parts: vec![
            MessagePart::Text("Describe this".to_string()),
            MessagePart::Part(Box::new(ContentPart {
                inline_data: Some(id),
                ..Default::default()
            })),
        ],
    };

    // Model supports "image" input
    let formatter: &'static dyn llm_secure_cli::llm::providers::payload_formatter::PayloadFormatter =
        Box::leak(Box::new(GenericPayloadFormatter));
    let input_mods: &'static [String] =
        Box::leak(Box::new(vec!["text".to_string(), "image".to_string()]));
    let msgs = MessageBuilder {
        formatter,
        model: "test-model",
        input_modalities: Some(input_mods),
        system_prompt: None,
        conversation: Box::leak(Box::new(vec![msg])),
        pending_data: Box::leak(Box::new(vec![])),
    }
    .build();

    let content = msgs[0]["content"].as_array().expect("Should be array");
    assert_eq!(content.len(), 2);
    assert_eq!(content[1]["type"], "image_url");
}

#[test]
fn test_image_skipped_when_modality_not_supported() {
    use std::collections::HashMap;
    let mut id = HashMap::new();
    id.insert(
        "mimeType".to_string(),
        Value::String("image/png".to_string()),
    );
    id.insert("data".to_string(), Value::String("base64data".to_string()));

    let msg = Message {
        role: Role::User,
        parts: vec![
            MessagePart::Text("Describe this".to_string()),
            MessagePart::Part(Box::new(ContentPart {
                inline_data: Some(id),
                ..Default::default()
            })),
        ],
    };

    // Model only supports "text" input (no "image")
    let formatter: &'static dyn llm_secure_cli::llm::providers::payload_formatter::PayloadFormatter =
        Box::leak(Box::new(GenericPayloadFormatter));
    let input_mods: &'static [String] = Box::leak(Box::new(vec!["text".to_string()]));
    let msgs = MessageBuilder {
        formatter,
        model: "test-model",
        input_modalities: Some(input_mods),
        system_prompt: None,
        conversation: Box::leak(Box::new(vec![msg])),
        pending_data: Box::leak(Box::new(vec![])),
    }
    .build();

    // Only text part should be present, image skipped
    assert_eq!(msgs[0]["content"], "Describe this");
}

#[test]
fn test_image_included_when_modality_info_unavailable() {
    // When input_modalities is None (unknown), images should be included (backward compat)
    use std::collections::HashMap;
    let mut id = HashMap::new();
    id.insert(
        "mimeType".to_string(),
        Value::String("image/png".to_string()),
    );
    id.insert("data".to_string(), Value::String("base64data".to_string()));

    let msg = Message {
        role: Role::User,
        parts: vec![MessagePart::Part(Box::new(ContentPart {
            inline_data: Some(id),
            ..Default::default()
        }))],
    };

    let msgs = builder(None, vec![msg], vec![]).build();
    let content = msgs[0]["content"].as_array().expect("Should be array");
    assert_eq!(content[0]["type"], "image_url");
}

// ===========================================================================
// Audio content formatting
// ===========================================================================

#[test]
fn test_audio_content_passed_through() {
    use std::collections::HashMap;
    let mut id = HashMap::new();
    id.insert(
        "mimeType".to_string(),
        Value::String("audio/mp3".to_string()),
    );
    id.insert("data".to_string(), Value::String("base64audio".to_string()));

    let msg = Message {
        role: Role::User,
        parts: vec![MessagePart::Part(Box::new(ContentPart {
            inline_data: Some(id),
            ..Default::default()
        }))],
    };

    let msgs = builder(None, vec![msg], vec![]).build();
    let content = msgs[0]["content"].as_array().expect("Should be array");
    assert_eq!(content[0]["type"], "input_audio");
    assert_eq!(content[0]["input_audio"]["data"], "base64audio");
    // format is derived from the mime type: "mp3"
    assert_eq!(content[0]["input_audio"]["format"], "mp3");
}
