use crate::llm::base::{BaseLlmClientData, LlmClient, ProviderSpec};
use crate::llm::models::{ClientState, ContentPart, DataSource, Message, MessagePart, Role};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use reqwest::Client;
use serde_json::json;
use std::collections::HashMap;

static HTTP_CLIENT: Lazy<Client> = Lazy::new(Client::new);

pub struct OpenAiClient {
    pub base: BaseLlmClientData,
    pub api_url: String,
}

impl OpenAiClient {
    pub fn new(model: &str, stdout: bool, raw: bool) -> Self {
        let spec = ProviderSpec {
            api_key_name: "api_key".to_string(),
            config_section: "openai".to_string(),
            pdf_as_base64: true,
        };
        let base = BaseLlmClientData::new(model, spec, stdout, raw);
        Self {
            base,
            api_url: "https://api.openai.com/v1/chat/completions".to_string(),
        }
    }

    fn build_messages(&self, data: &[DataSource]) -> Vec<serde_json::Value> {
        let mut messages = Vec::new();

        // Prepend system prompt if enabled
        if self.base.state.system_prompt_enabled {
            if let Some(sp) = &self.base.state.system_prompt {
                if !sp.is_empty() {
                    messages.push(json!({
                        "role": "system",
                        "content": sp
                    }));
                }
            }
        }

        for m in &self.base.state.conversation {
            let role = match m.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant | Role::Model => "assistant",
                Role::Tool => "tool",
            };

            let mut msg = json!({ "role": role });

            let mut text_content = String::new();
            let mut tool_calls = Vec::new();

            for part in &m.parts {
                match part {
                    MessagePart::Text(t) => text_content.push_str(t),
                    MessagePart::Part(cp) => {
                        if let Some(t) = &cp.text {
                            text_content.push_str(t);
                        }
                        if let Some(fc) = &cp.function_call {
                            tool_calls.push(json!({
                                "id": fc.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                                "type": "function",
                                "function": {
                                    "name": fc.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                                    "arguments": fc.get("arguments").map(|v| v.to_string()).unwrap_or_else(|| "{}".to_string())
                                }
                            }));
                        }
                        if let Some(fr) = &cp.function_response {
                            msg["tool_call_id"] = fr.get("id").cloned().unwrap_or(json!(""));
                            let response = fr.get("response").cloned().unwrap_or(json!(""));
                            let response_str = if let Some(s) = response.as_str() {
                                s.to_string()
                            } else {
                                response.to_string()
                            };
                            text_content.push_str(&response_str);
                        }
                    }
                }
            }

            if !text_content.is_empty() {
                msg["content"] = json!(text_content);
            } else if role == "tool" {
                msg["content"] = json!(""); // Tool messages must have content
            } else if role == "assistant" && !tool_calls.is_empty() {
                msg["content"] = json!(null);
            }

            if !tool_calls.is_empty() {
                msg["tool_calls"] = json!(tool_calls);
            }

            messages.push(msg);
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
}

#[async_trait]
impl LlmClient for OpenAiClient {
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
        let tool_schemas = crate::tools::registry::REGISTRY
            .lock()
            .unwrap()
            .get_tool_schemas();

        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(key) = &self.base.api_key {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", key).parse()?,
            );
        }

        let mut payload = json!({
            "model": self.base.state.model,
            "messages": messages,
        });

        if !tool_schemas.is_empty() {
            payload["tools"] = json!(tool_schemas
                .into_iter()
                .map(|s| {
                    json!({
                        "type": "function",
                        "function": s
                    })
                })
                .collect::<Vec<_>>());
        }

        log::debug!(
            "OpenAI Request Payload: {}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );

        let res = HTTP_CLIENT
            .post(&self.api_url)
            .headers(headers)
            .json(&payload)
            .send()
            .await?;

        let res_json: serde_json::Value = res.json().await?;
        log::debug!(
            "OpenAI Response: {}",
            serde_json::to_string_pretty(&res_json).unwrap_or_default()
        );

        if let Some(err) = res_json.get("error") {
            return Err(anyhow::anyhow!("OpenAI API error: {}", err));
        }

        let choice = &res_json["choices"][0];
        let message = &choice["message"];
        let text = message["content"].as_str().map(|s| s.to_string());

        let mut model_parts = Vec::new();
        if let Some(t) = text.clone() {
            model_parts.push(MessagePart::Text(t));
        }

        if let Some(tool_calls) = message["tool_calls"].as_array() {
            for tc in tool_calls {
                let id = tc["id"].as_str().unwrap_or("").to_string();
                let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
                let args_str = tc["function"]["arguments"]
                    .as_str()
                    .unwrap_or("{}")
                    .to_string();
                let args: HashMap<String, serde_json::Value> =
                    serde_json::from_str(&args_str).unwrap_or_default();

                let mut function_call = HashMap::new();
                function_call.insert("name".to_string(), json!(name));
                function_call.insert("arguments".to_string(), json!(args));
                function_call.insert("id".to_string(), json!(id));

                model_parts.push(MessagePart::Part(ContentPart {
                    text: None,
                    inline_data: None,
                    function_call: Some(function_call),
                    function_response: None,
                    thought: None,
                    thought_signature: None,
                    is_diagnostic: false,
                }));
            }
        }

        let model_msg = Message {
            role: Role::Assistant,
            parts: model_parts,
        };

        self.update_history(&data, model_msg);

        Ok((text, None))
    }
}
