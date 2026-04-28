use crate::llm::base::{BaseLlmClientData, LlmClient, ProviderSpec};
use crate::llm::models::{ClientState, ContentPart, DataSource, Message, MessagePart, Role};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::HashMap;

use once_cell::sync::Lazy;

static CLIENT: Lazy<reqwest::Client> = Lazy::new(crate::llm::base::create_http_client);

/// Generic OpenAI-compatible API client.
/// Supports any provider that follows the OpenAI Chat Completions API format.
pub struct OpenAiCompatibleClient {
    pub base: BaseLlmClientData,
    pub api_url: String,
    pub api_key: String,
    pub supports_tools: bool,
    pub supports_vision: bool,
}

impl OpenAiCompatibleClient {
    pub fn new(base_url: &str, api_key: &str, model: &str, stdout: bool, raw: bool) -> Self {
        let spec = ProviderSpec {
            api_key_name: "api_key".to_string(),
            config_section: "custom".to_string(),
            pdf_as_base64: true,
        };
        let base = BaseLlmClientData::new(model, spec, stdout, raw);
        Self {
            base,
            api_url: format!("{}/chat/completions", base_url.trim_end_matches('/')),
            api_key: api_key.to_string(),
            supports_tools: true,
            supports_vision: true,
        }
    }

    fn data_url(mime_type: &str, b64_data: &str) -> String {
        format!("data:{};base64,{}", mime_type, b64_data)
    }

    fn build_messages(&self, data: &[DataSource]) -> Vec<Value> {
        let mut messages = Vec::new();

        // Convert conversation history
        for m in &self.base.state.conversation {
            if m.role == Role::System {
                continue; // handled separately
            }

            if m.role == Role::Tool {
                // Tool results as individual function_result messages
                for part in &m.parts {
                    if let MessagePart::Part(cp) = part
                        && let Some(fr) = &cp.function_response
                    {
                        let tool_call_id = fr.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        let name = fr.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let response = fr.get("response").cloned().unwrap_or(json!(""));
                        let content = if let Some(s) = response.as_str() {
                            s.to_string()
                        } else {
                            response.to_string()
                        };

                        messages.push(json!({
                            "role": "tool",
                            "tool_call_id": tool_call_id,
                            "name": name,
                            "content": content
                        }));
                    }
                }
                continue;
            }

            let role = match m.role {
                Role::User => "user",
                Role::Assistant | Role::Model => "assistant",
                _ => "user",
            };

            let mut content_parts = Vec::new();

            for part in &m.parts {
                match part {
                    MessagePart::Text(t) => {
                        content_parts.push(json!({"type": "text", "text": t}));
                    }
                    MessagePart::Part(cp) => {
                        if let (Some(thought), _) = (&cp.thought, &cp.thought_signature) {
                            content_parts.push(
                                json!({"type": "text", "text": format!("[Thinking] {}", thought)}),
                            );
                        }
                        if let Some(t) = &cp.text
                            && !t.is_empty()
                            && !cp.is_diagnostic
                        {
                            content_parts.push(json!({"type": "text", "text": t}));
                        }
                        if let Some(fc) = &cp.function_call {
                            let id = fc.get("id").and_then(|v| v.as_str()).unwrap_or("");
                            let name = fc.get("name").and_then(|v| v.as_str()).unwrap_or("");
                            let args = fc.get("arguments").cloned().unwrap_or(json!({}));
                            content_parts.push(json!({
                                "type": "tool_call",
                                "tool_call": {
                                    "id": id,
                                    "type": "function",
                                    "function": {
                                        "name": name,
                                        "arguments": args.to_string()
                                    }
                                }
                            }));
                        }
                        if let Some(id) = &cp.inline_data {
                            let mime = id.get("mimeType").and_then(|v| v.as_str()).unwrap_or("");
                            let data = id.get("data").and_then(|v| v.as_str()).unwrap_or("");
                            if mime.starts_with("image/") || mime == "application/pdf" {
                                content_parts.push(json!({
                                    "type": "image_url",
                                    "image_url": {
                                        "url": Self::data_url(mime, data)
                                    }
                                }));
                            }
                        }
                    }
                }
            }

            if !content_parts.is_empty() {
                // Extract tool calls before consuming content_parts
                let tool_calls: Vec<Value> = content_parts
                    .iter()
                    .filter_map(|p| p.get("tool_call").cloned())
                    .collect();

                // For vision models, use array content; for non-vision, collapse to string
                let content = if self.supports_vision && content_parts.len() > 1 {
                    Value::Array(content_parts)
                } else {
                    content_parts
                        .into_iter()
                        .filter_map(|p| p.get("text").cloned())
                        .collect::<Vec<_>>()
                        .into()
                };
                let mut msg = json!({"role": role, "content": content});
                let role_clone = msg
                    .get("role")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let role2 = role_clone.as_str();
                if (role2 == "assistant" || role2 == "model") && !tool_calls.is_empty() {
                    msg["tool_calls"] = Value::Array(tool_calls);
                }
                messages.push(msg);
            }
        }

        // Add new user data
        let mut new_parts = Vec::new();
        for d in data {
            match d.content_type.as_str() {
                "text/plain" => {
                    new_parts
                        .push(json!({"type": "text", "text": d.content.as_str().unwrap_or("")}));
                }
                ct if ct.starts_with("image/") || ct == "application/pdf" => {
                    if let Some(b64) = d.content.as_str() {
                        if !self.supports_vision {
                            new_parts.push(json!({
                                "type": "text",
                                "text": format!("[Media omitted: {}. Vision not supported by this provider.]", ct)
                            }));
                        } else {
                            new_parts.push(json!({
                                "type": "image_url",
                                "image_url": {
                                    "url": Self::data_url(ct, b64)
                                }
                            }));
                        }
                    }
                }
                ct if ct.starts_with("audio/") || ct.starts_with("video/") => {
                    new_parts.push(json!({
                        "type": "text",
                        "text": format!("[Media omitted: {}. Audio/video not supported.]", ct)
                    }));
                }
                _ if d.is_file_or_url => {
                    if let Some(text) = d.content.as_str() {
                        new_parts.push(json!({"type": "text", "text": format!("[File content: {}]\n{}", 
                            d.metadata.get("filename").and_then(|v| v.as_str()).unwrap_or("unknown"),
                            text
                        )}));
                    }
                }
                _ => {
                    new_parts
                        .push(json!({"type": "text", "text": d.content.as_str().unwrap_or("")}));
                }
            }
        }

        if !new_parts.is_empty() {
            let content = if self.supports_vision && new_parts.len() > 1 {
                Value::Array(new_parts)
            } else {
                new_parts
                    .iter()
                    .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n")
                    .into()
            };
            messages.push(json!({"role": "user", "content": content}));
        }

        messages
    }

    fn build_tool_schemas(&self) -> Option<Vec<Value>> {
        if !self.supports_tools {
            return None;
        }
        let registry = crate::tools::registry::REGISTRY.lock().unwrap();
        let schemas = registry.get_tool_schemas();
        if schemas.is_empty() {
            None
        } else {
            Some(
                schemas
                    .into_iter()
                    .map(|s| {
                        json!({
                            "type": "function",
                            "function": s
                        })
                    })
                    .collect(),
            )
        }
    }
}

#[async_trait]
impl LlmClient for OpenAiCompatibleClient {
    fn get_state(&self) -> &ClientState {
        &self.base.state
    }
    fn get_state_mut(&mut self) -> &mut ClientState {
        &mut self.base.state
    }
    fn get_config_section(&self) -> &str {
        &self.base.config_section
    }

    async fn send(
        &mut self,
        data: Vec<DataSource>,
    ) -> anyhow::Result<(Option<String>, Option<String>)> {
        let messages = self.build_messages(&data);

        let mut body = json!({
            "model": self.base.state.model,
            "messages": messages,
            "max_tokens": 8192,
        });

        // Add system prompt
        if let Some(sp) = &self.base.state.system_prompt {
            body["messages"] = json!([json!({"role": "system", "content": sp}), messages]);
        }

        // Add tools if enabled
        if self.base.state.tools_enabled
            && let Some(tools) = self.build_tool_schemas()
        {
            body["tools"] = json!(tools);
            body["tool_choice"] = json!("auto");
        }

        let res = CLIENT
            .post(&self.api_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let resp: Value = res.json().await?;

        // Extract response
        let choice = resp["choices"][0].clone();
        let message = &choice["message"];
        let _finish_reason = choice["finish_reason"].as_str().unwrap_or("");

        let text = message
            .get("content")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Extract tool calls
        if let Some(tool_calls) = message.get("tool_calls").and_then(|v| v.as_array()) {
            let mut parts = Vec::new();

            if let Some(t) = &text
                && !t.is_empty()
            {
                parts.push(MessagePart::Text(t.clone()));
            }

            for tc in tool_calls {
                if tc.get("type").and_then(|v| v.as_str()) == Some("function") {
                    let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let func = &tc["function"];
                    let name = func.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let args_str = func
                        .get("arguments")
                        .and_then(|v| v.as_str())
                        .unwrap_or("{}");
                    let args: Value = serde_json::from_str(args_str).unwrap_or(json!({}));

                    let mut fc = HashMap::new();
                    fc.insert("id".to_string(), json!(id));
                    fc.insert("name".to_string(), json!(name));
                    fc.insert("arguments".to_string(), args);

                    parts.push(MessagePart::Part(ContentPart {
                        text: None,
                        inline_data: None,
                        function_call: Some(fc),
                        function_response: None,
                        thought: None,
                        thought_signature: None,
                        is_diagnostic: false,
                    }));
                }
            }

            let model_msg = Message {
                role: Role::Assistant,
                parts,
            };
            self.update_history(&data, model_msg);

            return Ok((text, None));
        }

        let model_msg = Message {
            role: Role::Assistant,
            parts: text
                .as_ref()
                .map(|t| vec![MessagePart::Text(t.clone())])
                .unwrap_or_default(),
        };
        self.update_history(&data, model_msg);

        Ok((text, None))
    }

    async fn send_as_verifier(
        &mut self,
        data: Vec<DataSource>,
        _tool_schema: Value,
    ) -> anyhow::Result<Value> {
        // Verifier mode: use tool_choice=none to guarantee structured output
        let messages = self.build_messages(&data);

        let mut body = json!({
            "model": self.base.state.model,
            "messages": messages,
            "max_tokens": 1024,
        });

        if let Some(sp) = &self.base.state.system_prompt {
            body["messages"] = json!([json!({"role": "system", "content": sp}), messages]);
        }

        // Force structured output via tool_choice=none + response_format if available
        let res = CLIENT
            .post(&self.api_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let resp: Value = res.json().await?;
        let content = resp["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("{}");

        // Try to parse as JSON
        match serde_json::from_str(content) {
            Ok(v) => Ok(v),
            Err(_) => Ok(json!({"safe": false, "reason": content})),
        }
    }
}
