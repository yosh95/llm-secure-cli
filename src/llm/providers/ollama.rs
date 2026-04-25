use crate::config::CONFIG_MANAGER;
use crate::llm::base::{BaseLlmClientData, LlmClient, ProviderSpec};
use crate::llm::models::{ClientState, ContentPart, DataSource, Message, MessagePart, Role};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde_json::json;
use std::collections::HashMap;

static AGENT: Lazy<ureq::Agent> = Lazy::new(|| ureq::Agent::config_builder().build().into());

pub struct OllamaClient {
    pub base: BaseLlmClientData,
    pub api_url: String,
}

impl OllamaClient {
    pub fn new(model: &str, stdout: bool, raw: bool) -> Self {
        let spec = ProviderSpec {
            api_key_name: "api_key".to_string(),
            config_section: "ollama".to_string(),
            pdf_as_base64: false,
        };
        let mut base = BaseLlmClientData::new(model, spec, stdout, raw);

        // Ollama might not need an API key, but BaseLlmClientData tries to get one.
        // If it's None, we might still want to proceed.
        if base.api_key.is_none() {
            base.api_key = Some("ollama".to_string());
        }

        let config = CONFIG_MANAGER.get_config();
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

                    // Check if this assistant message contains tool_calls
                    let tool_calls: Vec<serde_json::Value> = m
                        .parts
                        .iter()
                        .filter_map(|p| {
                            if let MessagePart::Part(cp) = p
                                && let Some(fc) = &cp.function_call
                            {
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
                                Some(json!({
                                    "id": call_id,
                                    "type": "function",
                                    "function": {
                                        "name": name,
                                        "arguments": arguments
                                    }
                                }))
                            } else {
                                None
                            }
                        })
                        .collect();

                    let text = m.get_text(false);

                    if !tool_calls.is_empty() {
                        // Assistant message with tool calls
                        let mut msg = json!({
                            "role": "assistant",
                            "tool_calls": tool_calls
                        });
                        if !text.is_empty() {
                            msg["content"] = json!(text);
                        }
                        messages.push(msg);
                    } else if !text.is_empty() {
                        messages.push(json!({
                            "role": role,
                            "content": text
                        }));
                    }
                }
            }
        }

        for d in data {
            if d.content_type == "text/plain" {
                messages.push(json!({
                    "role": "user",
                    "content": d.content.as_str().unwrap_or("")
                }));
            }
        }

        messages
    }

    /// Build an OpenAI-compatible tools array from the global tool registry.
    fn build_tools(&self) -> Vec<serde_json::Value> {
        if !self.base.state.tools_enabled {
            return Vec::new();
        }
        let registry = crate::tools::registry::REGISTRY.lock().unwrap();
        registry
            .get_tool_schemas()
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
    fn post_request(&self, payload: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let api_key = self.base.api_key.clone();
        let api_url = self.api_url.clone();
        let is_cloud = self.is_cloud_endpoint();

        let res_result = tokio::task::block_in_place(|| {
            let rt = tokio::runtime::Handle::current();
            rt.block_on(async {
                tokio::task::spawn_blocking(move || {
                    let mut req = AGENT.post(&api_url);
                    if let Some(key) = api_key {
                        // For cloud endpoints, always send the Authorization header.
                        // For local endpoints, skip the dummy "ollama"/"local_bypass" keys.
                        let skip = !is_cloud && (key == "ollama" || key == "local_bypass");
                        if !skip {
                            req = req.header("Authorization", format!("Bearer {}", key));
                        }
                    }
                    req.send_json(payload)
                })
                .await
            })
        });

        let res = res_result??;

        let status = res.status();
        let res_json: serde_json::Value = res.into_body().read_json().unwrap_or_default();

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

    async fn send(
        &mut self,
        data: Vec<DataSource>,
    ) -> anyhow::Result<(Option<String>, Option<String>)> {
        let messages = self.build_messages(&data);
        let tools = self.build_tools();

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

        let res_json = self.post_request(payload)?;

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

        let res_json = self.post_request(payload)?;

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
