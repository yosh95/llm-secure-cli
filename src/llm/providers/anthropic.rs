use crate::llm::base::{BaseLlmClientData, LlmClient, ProviderSpec};
use crate::llm::models::{ClientState, ContentPart, DataSource, Message, MessagePart, Role};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::HashMap;

static HTTP_CLIENT: Lazy<Client> = Lazy::new(Client::new);

pub struct ClaudeClient {
    pub base: BaseLlmClientData,
    pub api_url: String,
}

impl ClaudeClient {
    pub fn new(model: &str, stdout: bool, raw: bool) -> Self {
        let spec = ProviderSpec {
            api_key_name: "api_key".to_string(),
            config_section: "anthropic".to_string(),
            pdf_as_base64: true,
        };
        let base = BaseLlmClientData::new(model, spec, stdout, raw);
        Self {
            base,
            api_url: "https://api.anthropic.com/v1/messages".to_string(),
        }
    }

    /// Build Anthropic Messages API message array from conversation history + new data.
    fn build_messages(&self, data: &[DataSource]) -> Vec<Value> {
        let mut messages = Vec::new();

        for m in &self.base.state.conversation {
            // Tool results are sent as a user message with tool_result blocks
            if m.role == Role::Tool {
                let mut tool_content = Vec::new();
                for part in &m.parts {
                    if let MessagePart::Part(cp) = part {
                        if let Some(fr) = &cp.function_response {
                            let tool_use_id = fr.get("id").and_then(|v| v.as_str()).unwrap_or("");
                            let response = fr.get("response").cloned().unwrap_or(json!(""));
                            let response_str = if let Some(s) = response.as_str() {
                                s.to_string()
                            } else {
                                response.to_string()
                            };
                            tool_content.push(json!({
                                "type": "tool_result",
                                "tool_use_id": tool_use_id,
                                "content": response_str
                            }));
                        }
                    }
                }
                if !tool_content.is_empty() {
                    messages.push(json!({
                        "role": "user",
                        "content": tool_content
                    }));
                }
                continue;
            }

            let role = match m.role {
                Role::Assistant | Role::Model => "assistant",
                _ => "user",
            };

            let mut content = Vec::new();
            for part in &m.parts {
                match part {
                    MessagePart::Text(t) => {
                        if !t.is_empty() {
                            content.push(json!({ "type": "text", "text": t }));
                        }
                    }
                    MessagePart::Part(cp) => {
                        // Thinking block
                        if let (Some(thought), Some(sig)) = (&cp.thought, &cp.thought_signature) {
                            content.push(json!({
                                "type": "thinking",
                                "thinking": thought,
                                "signature": sig
                            }));
                        } else if let Some(thought) = &cp.thought {
                            content.push(json!({
                                "type": "thinking",
                                "thinking": thought,
                                "signature": ""
                            }));
                        }

                        // Text block
                        if let Some(t) = &cp.text {
                            if !t.is_empty() && !cp.is_diagnostic {
                                content.push(json!({ "type": "text", "text": t }));
                            }
                        }

                        // Image / PDF inline data
                        if let Some(inline) = &cp.inline_data {
                            let mime = inline
                                .get("mimeType")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let data_b64 =
                                inline.get("data").and_then(|v| v.as_str()).unwrap_or("");
                            if mime.starts_with("image/") {
                                content.push(json!({
                                    "type": "image",
                                    "source": {
                                        "type": "base64",
                                        "media_type": mime,
                                        "data": data_b64
                                    }
                                }));
                            } else if mime == "application/pdf" {
                                content.push(json!({
                                    "type": "document",
                                    "source": {
                                        "type": "base64",
                                        "media_type": mime,
                                        "data": data_b64
                                    }
                                }));
                            }
                        }

                        // Tool use block (assistant → tool call)
                        if let Some(fc) = &cp.function_call {
                            let id = fc.get("id").and_then(|v| v.as_str()).unwrap_or("");
                            let name = fc.get("name").and_then(|v| v.as_str()).unwrap_or("");
                            let input = fc.get("arguments").cloned().unwrap_or_else(|| json!({}));
                            content.push(json!({
                                "type": "tool_use",
                                "id": id,
                                "name": name,
                                "input": input
                            }));
                        }
                    }
                }
            }

            if !content.is_empty() {
                messages.push(json!({ "role": role, "content": content }));
            }
        }

        // Append the new user input data
        let mut new_content = Vec::new();
        for d in data {
            if d.content_type == "text/plain" {
                new_content.push(json!({
                    "type": "text",
                    "text": d.content.as_str().unwrap_or("")
                }));
            } else if d.content_type.starts_with("image/") {
                new_content.push(json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": d.content_type,
                        "data": d.content.as_str().unwrap_or("")
                    }
                }));
            } else if d.content_type == "application/pdf" {
                new_content.push(json!({
                    "type": "document",
                    "source": {
                        "type": "base64",
                        "media_type": d.content_type,
                        "data": d.content.as_str().unwrap_or("")
                    }
                }));
            }
        }
        if !new_content.is_empty() {
            messages.push(json!({ "role": "user", "content": new_content }));
        }

        messages
    }
}

#[async_trait]
impl LlmClient for ClaudeClient {
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

        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(key) = &self.base.api_key {
            headers.insert("x-api-key", key.parse()?);
        }
        headers.insert("anthropic-version", "2023-06-01".parse()?);
        headers.insert("content-type", "application/json".parse()?);

        // Build tool schemas (Anthropic format)
        let tool_schemas = crate::tools::registry::REGISTRY
            .lock()
            .unwrap()
            .get_tool_schemas_anthropic();

        let mut payload = json!({
            "model": self.base.state.model,
            "max_tokens": 8192,
            "messages": messages,
        });

        // System prompt
        if let Some(sp) = &self.base.state.system_prompt {
            if !sp.is_empty() {
                payload["system"] = json!([{
                    "type": "text",
                    "text": sp,
                    "cache_control": { "type": "ephemeral" }
                }]);
            }
        }

        // Tools
        if self.base.state.tools_enabled {
            // Include native web_search tool if brave_search is not registered
            let registry = crate::tools::registry::REGISTRY.lock().unwrap();
            let has_brave = registry.tools.contains_key("brave_search");
            drop(registry);

            let mut tools = tool_schemas;
            if !has_brave {
                // Prepend Anthropic native web search
                let mut all_tools = vec![json!({
                    "type": "web_search_20260209",
                    "name": "web_search"
                })];
                all_tools.extend(tools);
                tools = all_tools;
            }
            if !tools.is_empty() {
                payload["tools"] = json!(tools);
            }
        }

        log::debug!(
            "Anthropic Request Payload: {}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );

        let res = HTTP_CLIENT
            .post(&self.api_url)
            .headers(headers)
            .json(&payload)
            .send()
            .await?;

        let status = res.status();
        let res_json: Value = res.json().await?;
        log::debug!(
            "Anthropic Response ({}): {}",
            status,
            serde_json::to_string_pretty(&res_json).unwrap_or_default()
        );

        if !status.is_success() {
            if let Some(err) = res_json.get("error") {
                return Err(anyhow::anyhow!("Anthropic API error ({}): {}", status, err));
            } else {
                return Err(anyhow::anyhow!(
                    "Anthropic API error ({}): {}",
                    status,
                    res_json
                ));
            }
        }

        let mut full_text = String::new();
        let mut thought_text = String::new();
        let mut model_parts: Vec<MessagePart> = Vec::new();

        if let Some(content_blocks) = res_json["content"].as_array() {
            for block in content_blocks {
                let block_type = block["type"].as_str().unwrap_or("");
                match block_type {
                    "text" => {
                        if let Some(text) = block["text"].as_str() {
                            full_text.push_str(text);
                            model_parts.push(MessagePart::Part(ContentPart {
                                text: Some(text.to_string()),
                                inline_data: None,
                                function_call: None,
                                function_response: None,
                                thought: None,
                                thought_signature: None,
                                is_diagnostic: false,
                            }));
                        }
                    }
                    "thinking" => {
                        if let Some(thought) = block["thinking"].as_str() {
                            thought_text.push_str(thought);
                            let sig = block["signature"].as_str().map(|s| s.to_string());
                            model_parts.push(MessagePart::Part(ContentPart {
                                text: None,
                                inline_data: None,
                                function_call: None,
                                function_response: None,
                                thought: Some(thought.to_string()),
                                thought_signature: sig,
                                is_diagnostic: false,
                            }));
                        }
                    }
                    "tool_use" => {
                        let id = block["id"].as_str().unwrap_or("").to_string();
                        let name = block["name"].as_str().unwrap_or("").to_string();
                        let input = block["input"].clone();

                        let mut fc: HashMap<String, Value> = HashMap::new();
                        fc.insert("id".to_string(), json!(id));
                        fc.insert("name".to_string(), json!(name));
                        fc.insert("arguments".to_string(), input);

                        model_parts.push(MessagePart::Part(ContentPart {
                            text: None,
                            inline_data: None,
                            function_call: Some(fc),
                            function_response: None,
                            thought: None,
                            thought_signature: None,
                            is_diagnostic: false,
                        }));
                    }
                    "server_tool_use" | "web_search_tool_result" => {
                        // Diagnostic: record but don't surface as primary text
                        let diag_text = format!("[{}]", block_type);
                        model_parts.push(MessagePart::Part(ContentPart {
                            text: Some(diag_text),
                            inline_data: None,
                            function_call: None,
                            function_response: None,
                            thought: None,
                            thought_signature: None,
                            is_diagnostic: true,
                        }));
                    }
                    _ => {}
                }
            }
        }

        // Always record the assistant message (including tool_use blocks)
        let model_msg = Message {
            role: Role::Assistant,
            parts: model_parts,
        };

        self.update_history(&data, model_msg);

        Ok((
            if full_text.is_empty() {
                None
            } else {
                Some(full_text)
            },
            if thought_text.is_empty() {
                None
            } else {
                Some(thought_text)
            },
        ))
    }
}
