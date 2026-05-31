//! Parses OpenAI-compatible API responses into internal message parts.
//!
//! Extracted from the main client to keep the HTTP ↔ model mapping logic
//! independently testable.

use crate::llm::models::MessagePart;
use serde_json::Value;
use std::collections::HashMap;

/// Parsed content extracted from a single API response.
pub struct ParsedResponse {
    /// Plain text content (may be `None` when only tool calls or media are present).
    pub text: Option<String>,
    /// Structured message parts (text, tool calls, inline media).
    pub message_parts: Vec<MessagePart>,
}

/// Parse the "choices\[0\].message" object from an OpenAI-compatible chat completion
/// response into a `ParsedResponse`.
#[must_use]
pub fn parse_assistant_message(msg: &Value) -> ParsedResponse {
    let text = msg["content"]
        .as_str()
        .map(std::string::ToString::to_string);

    let mut message_parts = Vec::new();
    if let Some(t) = &text {
        message_parts.push(MessagePart::Text(t.clone()));
    }

    // ── inline multimodal content (content array) ─────────────────────────
    parse_multimodal_content_array(msg, &mut message_parts);

    // ── top-level media fields (images / videos / audios) ─────────────────
    parse_top_level_media_fields(msg, &mut message_parts);

    // ── tool calls ────────────────────────────────────────────────────────
    parse_tool_calls(msg, &mut message_parts);

    ParsedResponse {
        text,
        message_parts,
    }
}

// ── private helpers ────────────────────────────────────────────────────────

fn parse_multimodal_content_array(msg: &Value, parts: &mut Vec<MessagePart>) {
    let array = match msg.get("content").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return,
    };

    for part in array {
        let p_type = match part.get("type").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => continue,
        };
        match p_type {
            "image_url" => {
                if let Some(b64) = extract_data_url_b64(
                    part.get("image_url")
                        .and_then(|v| v.get("url"))
                        .and_then(|v| v.as_str()),
                ) {
                    parts.push(inline_data_part("image/png", b64));
                } else if let Some(b64) = part
                    .get("image_url")
                    .and_then(|v| v.get("b64_json"))
                    .and_then(|v| v.as_str())
                {
                    // Some providers return b64_json instead of url
                    parts.push(inline_data_part("image/png", b64));
                }
            }
            "input_audio" => {
                if let Some(audio_data) = part.get("input_audio") {
                    let b64 = audio_data
                        .get("data")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();
                    let format = audio_data
                        .get("format")
                        .and_then(|v| v.as_str())
                        .unwrap_or("mp3");
                    if !b64.is_empty() {
                        parts.push(inline_data_part(&format!("audio/{format}"), b64));
                    }
                }
            }
            _ => {}
        }
    }
}

fn parse_top_level_media_fields(msg: &Value, parts: &mut Vec<MessagePart>) {
    let media_fields = ["images", "videos", "audios"];
    for field in media_fields {
        let array = match msg.get(field).and_then(|v| v.as_array()) {
            Some(a) => a,
            None => continue,
        };
        for part in array {
            let url = part
                .get("image_url") // Recraft uses this even for videos
                .and_then(|v| v.get("url"))
                .and_then(|v| v.as_str())
                .or_else(|| part.get("url").and_then(|v| v.as_str()))
                .or_else(|| part.as_str()); // Sometimes it's just a string array

            if let Some(b64) = extract_data_url_b64(url) {
                let mime = match field {
                    "videos" => "video/mp4",
                    "audios" => "audio/mpeg",
                    _ => "image/png",
                };
                parts.push(inline_data_part(mime, b64));
            }
        }
    }
}

fn parse_tool_calls(msg: &Value, parts: &mut Vec<MessagePart>) {
    let tool_calls = match msg.get("tool_calls").and_then(|v| v.as_array()) {
        Some(tc) => tc,
        None => return,
    };

    for tc in tool_calls {
        let mut fc = HashMap::new();
        fc.insert("id".to_string(), tc["id"].clone());
        fc.insert("name".to_string(), tc["function"]["name"].clone());
        let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
        let args: Value = serde_json::from_str(args_str).unwrap_or(serde_json::json!({}));
        fc.insert("arguments".to_string(), args);
        parts.push(MessagePart::Part(Box::new(
            crate::llm::models::ContentPart {
                function_call: Some(fc),
                ..Default::default()
            },
        )));
    }
}

// ── tiny helpers ───────────────────────────────────────────────────────────

/// If `url` is a `data:` URI, extract the base64 payload after the comma.
fn extract_data_url_b64(url: Option<&str>) -> Option<&str> {
    let url = url?;
    if !url.starts_with("data:") {
        return None;
    }
    url.split_once(',').map(|x| x.1)
}

fn inline_data_part(mime_type: &str, b64: &str) -> MessagePart {
    let mut id = HashMap::new();
    id.insert("mimeType".to_string(), Value::String(mime_type.to_string()));
    id.insert("data".to_string(), Value::String(b64.to_string()));
    MessagePart::Part(Box::new(crate::llm::models::ContentPart {
        inline_data: Some(id),
        ..Default::default()
    }))
}
