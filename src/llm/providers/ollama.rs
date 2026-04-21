use crate::config::CONFIG_MANAGER;
use crate::llm::base::{BaseLlmClientData, LlmClient, ProviderSpec};
use crate::llm::models::{ClientState, DataSource, Role};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use reqwest::Client;
use serde_json::json;

static HTTP_CLIENT: Lazy<Client> = Lazy::new(Client::new);

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
            messages.push(json!({
                "role": role,
                "content": m.get_text(false)
            }));
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

        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(key) = &self.base.api_key {
            if key != "ollama" && key != "local_bypass" {
                headers.insert(
                    reqwest::header::AUTHORIZATION,
                    format!("Bearer {}", key).parse()?,
                );
            }
        }

        let payload = json!({
            "model": self.base.state.model,
            "messages": messages,
        });

        log::debug!(
            "Ollama Request Payload: {}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );

        let res = HTTP_CLIENT
            .post(&self.api_url)
            .headers(headers)
            .json(&payload)
            .send()
            .await?;

        let status = res.status();
        let res_json: serde_json::Value = res.json().await?;
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

        let choices = res_json.get("choices").and_then(|v| v.as_array());
        let Some(choice) = choices.and_then(|c| c.first()) else {
            return Err(anyhow::anyhow!("Invalid response from Ollama: no choices"));
        };

        let text = choice["message"]["content"].as_str().map(|s| s.to_string());

        let model_msg = crate::llm::models::Message {
            role: Role::Assistant,
            parts: vec![crate::llm::models::MessagePart::Text(
                text.clone().unwrap_or_default(),
            )],
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

        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(key) = &self.base.api_key {
            if key != "ollama" && key != "local_bypass" {
                headers.insert(
                    reqwest::header::AUTHORIZATION,
                    format!("Bearer {}", key).parse()?,
                );
            }
        }

        let res = HTTP_CLIENT
            .post(&self.api_url)
            .headers(headers)
            .json(&payload)
            .send()
            .await?;

        if !res.status().is_success() {
            let res_json: serde_json::Value = res.json().await?;
            return Err(anyhow::anyhow!("Ollama verifier error: {}", res_json));
        }

        let res_json: serde_json::Value = res.json().await?;

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
