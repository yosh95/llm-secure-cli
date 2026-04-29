use crate::config::ConfigManager;
use crate::llm::base::{BaseLlmClientData, LlmClient, ProviderSpec};
use crate::llm::models::{ClientState, ContentPart, DataSource, Message, MessagePart, Role};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde_json::json;
use std::collections::HashMap;

static CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .build()
        .expect("Failed to create reqwest client")
});

pub struct OllamaClient {
    pub base: BaseLlmClientData,
    pub api_url: String,
}

impl OllamaClient {
    pub fn new(config_manager: &ConfigManager, model: &str, stdout: bool, raw: bool) -> Self {
        let spec = ProviderSpec {
            api_key_name: "api_key".to_string(),
            config_section: "ollama".to_string(),
            pdf_as_base64: false,
        };
        let mut base = BaseLlmClientData::new(config_manager, model, spec, stdout, raw);

        // Ollama might not need an API key, but BaseLlmClientData tries to get one.
        // If it's None, we might still want to proceed.
        if base.api_key.is_none() {
            base.api_key = Some("ollama".to_string());
        }

        let config = config_manager.get_config();
        let api_url = config
            .providers
            .get("ollama")
            .and_then(|p| p.api_url.clone())
            .unwrap_or_else(|| "http://localhost:11434/v1/chat/completions".to_string());

        Self { base, api_url }
    }

    /// Returns true when the configured endpoint is a remote (cloud) URL,
    /// i.e. it is NOT localhost / 127.0.0.1.
    fn is_cloud_endpoint(&self) -> bool {
        !self.api_url.contains("localhost") && !self.api_url.contains("127.0.0.1")
    }

    fn data_url(mime_type: &str, b64_data: &str) -> String {
        format!("data:{};base64,{}", mime_type, b64_data)
    }

    fn build_messages(&self, data: &[DataSource]) -> Vec<serde_json::Value> {
        let mut messages = Vec::new();

        // Prepend system prompt if enabled
        if let Some(sp) = self.base.state.get_effective_system_prompt() {
            messages.push(json!({
                "role": "system",
                "content": sp
            }));
        }

        for m in &self.base.state.conversation {
            match m.role {
                Role::Tool => {
                    // OpenAI-compatible tool result messages
                    for part in &m.parts {
                        if let MessagePart::Part(cp) = part
                            && let Some(fr) = &cp.function_response
                        {
                            let call_id = fr
                                .get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let response = fr.get("response").cloned().unwrap_or(json!(""));
                            let output = response
                                .as_str()
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| response.to_string());

                            messages.push(json!({
                                "role": "tool",
                                "tool_call_id": call_id,
                                "content": output
                            }));
                        }
                    }
                }
                _ => {
                    let role = match m.role {
                        Role::System => "system",
                        Role::User => "user",
                        Role::Assistant | Role::Model => "assistant",
                        Role::Tool => unreachable!(),
                    };

                    let mut content_parts = Vec::new();
                    let mut tool_calls = Vec::new();

                    for part in &m.parts {
                        match part {
                            MessagePart::Text(t) => {
                                if !t.is_empty() {
                                    content_parts.push(json!({"type": "text", "text": t}));
                                }
                            }
                            MessagePart::Part(cp) => {
                                if let Some(t) = &cp.text
                                    && !t.is_empty()
                                    && !cp.is_diagnostic
                                {
                                    content_parts.push(json!({"type": "text", "text": t}));
                                }

                                if let Some(inline) = &cp.inline_data {
                                    let mime = inline
                                        .get("mimeType")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    let b64 =
                                        inline.get("data").and_then(|v| v.as_str()).unwrap_or("");
                                    if !mime.is_empty()
                                        && !b64.is_empty()
                                        && (mime.starts_with("image/") || mime == "application/pdf")
                                    {
                                        content_parts.push(json!({
                                            "type": "image_url",
                                            "image_url": {
                                                "url": Self::data_url(mime, b64)
                                            }
                                        }));
                                    }
                                }

                                if let Some(fc) = &cp.function_call {
                                    let call_id = fc
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let name = fc
                                        .get("name")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let arguments = fc
                                        .get("arguments")
                                        .map(|v| {
                                            v.as_str()
                                                .map(|s| s.to_string())
                                                .unwrap_or_else(|| v.to_string())
                                        })
                                        .unwrap_or_else(|| "{}".to_string());
                                    tool_calls.push(json!({
                                        "id": call_id,
                                        "type": "function",
                                        "function": {
                                            "name": name,
                                            "arguments": arguments
                                        }
                                    }));
                                }
                            }
                        }
                    }

                    if !tool_calls.is_empty() {
                        let mut msg = json!({
                            "role": "assistant",
                            "tool_calls": tool_calls
                        });
                        if !content_parts.is_empty() {
                            // If we have text along with tool calls
                            let text: String = content_parts
                                .iter()
                                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                                .collect::<Vec<_>>()
                                .join("\n");
                            if !text.is_empty() {
                                msg["content"] = json!(text);
                            }
                        }
                        messages.push(msg);
                    } else if !content_parts.is_empty() {
                        // For vision-enabled models on Ollama/OpenAI-compatible endpoints,
                        // we can send an array of parts.
                        let content =
                            if content_parts.len() == 1 && content_parts[0]["type"] == "text" {
                                content_parts[0]["text"].clone()
                            } else {
                                json!(content_parts)
                            };

                        messages.push(json!({
                            "role": role,
                            "content": content
                        }));
                    }
                }
            }
        }

        // Add new data from data source
        let mut new_parts = Vec::new();
        for d in data {
            if d.content_type == "text/plain" {
                new_parts.push(json!({
                    "type": "text",
                    "text": d.content.as_str().unwrap_or("")
                }));
            } else if (d.content_type.starts_with("image/") || d.content_type == "application/pdf")
                && let Some(b64) = d.content.as_str()
            {
                new_parts.push(json!({
                    "type": "image_url",
                    "image_url": {
                        "url": Self::data_url(&d.content_type, b64)
                    }
                }));
            }
        }

        if !new_parts.is_empty() {
            let content = if new_parts.len() == 1 && new_parts[0]["type"] == "text" {
                new_parts[0]["text"].clone()
            } else {
                json!(new_parts)
            };
            messages.push(json!({
                "role": "user",
                "content": content
            }));
        }

        messages
    }

    /// Build an OpenAI-compatible tools array from the passed schemas.
    fn build_tools(&self, tool_schemas: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
        if !self.base.state.tools_enabled {
            return Vec::new();
        }
        tool_schemas
            .into_iter()
            .map(|s| {
                json!({
                    "type": "function",
                    "function": {
                        "name": s.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                        "description": s.get("description").and_then(|v| v.as_str()).unwrap_or(""),
                        "parameters": s.get("parameters").cloned().unwrap_or_else(|| json!({"type":"object","properties":{}}))
                    }
                })
            })
            .collect()
    }

    /// Send a request to the Ollama-compatible OpenAI endpoint and return the raw JSON response.
    async fn post_request(&self, payload: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let api_key = self.base.api_key.clone();
        let api_url = self.api_url.clone();
        let is_cloud = self.is_cloud_endpoint();

        let mut req = CLIENT.post(&api_url);
        if let Some(key) = api_key {
            // For cloud endpoints, always send the Authorization header.
            // For local endpoints, skip the dummy "ollama"/"local_bypass" keys.
            let skip = !is_cloud && (key == "ollama" || key == "local_bypass");
            if !skip {
                req = req.header("Authorization", format!("Bearer {}", key));
            }
        }
        let res = req.json(&payload).send().await?;

        let status = res.status();
        let res_json: serde_json::Value = res.json().await.unwrap_or_default();

        log::debug!(
            "Ollama Response ({}): {}",
            status,
            serde_json::to_string_pretty(&res_json).unwrap_or_default()
        );

        if !status.is_success() {
            if let Some(err) = res_json.get("error") {
                return Err(anyhow::anyhow!("Ollama API error ({}): {}", status, err));
            } else {
                return Err(anyhow::anyhow!(
                    "Ollama API error ({}): {}",
                    status,
                    res_json
                ));
            }
        }

        Ok(res_json)
    }
}

#[async_trait]
impl LlmClient for OllamaClient {
    fn get_state(&self) -> &ClientState {
        &self.base.state
    }
    fn get_state_mut(&mut self) -> &mut ClientState {
        &mut self.base.state
    }
    fn get_config_section(&self) -> &str {
        &self.base.config_section
    }
    fn should_send_pdf_as_base64(&self) -> bool {
        false
    }

    async fn send(
        &mut self,
        data: Vec<DataSource>,
        tool_schemas: Vec<serde_json::Value>,
    ) -> anyhow::Result<(Option<String>, Option<String>)> {
        let messages = self.build_messages(&data);
        let tools = self.build_tools(tool_schemas);

        let mut payload = json!({
            "model": self.base.state.model,
            "messages": messages,
        });

        if !tools.is_empty() {
            payload["tools"] = json!(tools);
        }

        log::debug!(
            "Ollama Request Payload: {}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );

        let res_json = self.post_request(payload).await?;

        let choices = res_json.get("choices").and_then(|v| v.as_array());
        let Some(choice) = choices.and_then(|c| c.first()) else {
            return Err(anyhow::anyhow!("Invalid response from Ollama: no choices"));
        };

        let message = &choice["message"];

        // Extract plain text content (may be null when model only returns tool_calls)
        let text = message["content"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        // --- Parse tool_calls (OpenAI-compatible format) ---
        let mut model_parts: Vec<MessagePart> = Vec::new();

        if let Some(text_str) = &text {
            model_parts.push(MessagePart::Text(text_str.clone()));
        }

        if let Some(tool_calls) = message["tool_calls"].as_array() {
            for tc in tool_calls {
                let call_id = tc
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
                let args_raw = tc["function"]["arguments"]
                    .as_str()
                    .unwrap_or("{}")
                    .to_string();

                // Parse argument string into a JSON object so the executor can use it directly.
                let args_val: serde_json::Value =
                    serde_json::from_str(&args_raw).unwrap_or_else(|_| json!({}));

                let mut fc_map: HashMap<String, serde_json::Value> = HashMap::new();
                fc_map.insert("id".to_string(), json!(call_id));
                fc_map.insert("name".to_string(), json!(name));
                fc_map.insert("arguments".to_string(), args_val);

                model_parts.push(MessagePart::Part(ContentPart {
                    text: None,
                    inline_data: None,
                    function_call: Some(fc_map),
                    function_response: None,
                    thought: None,
                    thought_signature: None,
                    is_diagnostic: false,
                }));
            }
        }

        // Build the assistant message and push to conversation history
        let model_msg = Message {
            role: Role::Assistant,
            parts: if model_parts.is_empty() {
                vec![MessagePart::Text(String::new())]
            } else {
                model_parts
            },
        };
        self.update_history(&data, model_msg);

        Ok((text, None))
    }

    async fn send_as_verifier(
        &mut self,
        data: Vec<DataSource>,
        tool_schema: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let messages = self.build_messages(&data);
        let tool_name = tool_schema["name"].as_str().unwrap_or("verify").to_string();

        let payload = json!({
            "model": self.base.state.model,
            "messages": messages,
            "tools": [{
                "type": "function",
                "function": tool_schema
            }],
            "tool_choice": {
                "type": "function",
                "function": { "name": tool_name }
            },
            "stream": false
        });

        let res_json = self.post_request(payload).await?;

        // Try to extract tool calls from Ollama response (OpenAI compatible format)
        let tool_call = res_json["choices"][0]["message"]["tool_calls"][0]["function"]
            .as_object()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "No tool call found in Ollama response. Ensure the model supports tools."
                )
            })?;

        let args_val = &tool_call["arguments"];
        if args_val.is_string() {
            let args_str = args_val.as_str().unwrap();
            Ok(serde_json::from_str(args_str)?)
        } else {
            Ok(args_val.clone())
        }
    }
}
