use crate::config::ConfigManager;
use crate::llm::base::{BaseLlmClientData, LlmClient, ProviderSpec, create_http_client};
use crate::llm::models::{ClientState, DataSource, Message, Role};
use async_trait::async_trait;
use serde_json::{Value, json};

use super::message_builder::MessageBuilder;
use super::{payload_formatter, response_parser};
use anyhow::Context;

// Re-export commonly-used items so existing imports stay unchanged.
pub use payload_formatter::{
    GenericPayloadFormatter, HighFeaturePayloadFormatter, PayloadFormatter,
};

// ── Client struct + Builder ────────────────────────────────────────────────

pub struct OpenAiCompatibleClient {
    pub base: BaseLlmClientData,
    pub api_url: String,
    pub api_key: String,
    pub http_client: reqwest::Client,
    pub formatter: Box<dyn PayloadFormatter>,
    pub supports_tools: bool,
}

pub struct OpenAiCompatibleClientBuilder<'a> {
    config_manager: &'a ConfigManager,
    provider_name: String,
    api_url: String,
    api_key: String,
    model: String,
    stdout: bool,
    raw: bool,
    formatter: Option<Box<dyn PayloadFormatter>>,
    supports_tools: Option<bool>,
}

impl<'a> OpenAiCompatibleClientBuilder<'a> {
    pub fn new(config_manager: &'a ConfigManager) -> Self {
        Self {
            config_manager,
            provider_name: String::new(),
            api_url: String::new(),
            api_key: String::new(),
            model: String::new(),
            stdout: false,
            raw: false,
            formatter: None,
            supports_tools: None,
        }
    }

    pub fn provider_name(mut self, name: &str) -> Self {
        self.provider_name = name.to_string();
        self
    }

    pub fn api_url(mut self, url: &str) -> Self {
        self.api_url = url.to_string();
        self
    }

    pub fn api_key(mut self, key: &str) -> Self {
        self.api_key = key.to_string();
        self
    }

    pub fn model(mut self, model: &str) -> Self {
        self.model = model.to_string();
        self
    }

    pub fn stdout(mut self, stdout: bool) -> Self {
        self.stdout = stdout;
        self
    }

    pub fn raw(mut self, raw: bool) -> Self {
        self.raw = raw;
        self
    }

    pub fn formatter(mut self, formatter: Box<dyn PayloadFormatter>) -> Self {
        self.formatter = Some(formatter);
        self
    }

    /// Override the tool-support detection for this model.
    ///
    /// When set to `Some(false)`, tool definitions will never be sent in requests
    /// even if the model list cache would indicate otherwise.  When `None` (the
    /// default), the builder will look up the model in the cache to decide.
    pub fn supports_tools(mut self, supports: Option<bool>) -> Self {
        self.supports_tools = supports;
        self
    }

    pub fn build(self) -> anyhow::Result<OpenAiCompatibleClient> {
        let spec = ProviderSpec {
            api_key_name: "api_key".to_string(),
            config_section: self.provider_name.clone(),
            pdf_as_base64: true, // We handle the decision in the formatter
        };
        let base = BaseLlmClientData::new(
            self.config_manager,
            &self.model,
            spec,
            self.stdout,
            self.raw,
        );

        let api_url = if self.api_url.ends_with("/chat/completions") {
            self.api_url
        } else {
            format!("{}/chat/completions", self.api_url.trim_end_matches('/'))
        };

        // Determine tool support:
        //   1. Explicit override from builder (Some(val))
        //   2. Cache lookup (if supported_parameters metadata exists)
        //   3. Default to `true` (backward compatibility)
        let supports_tools = self.supports_tools.unwrap_or_else(|| {
            self.config_manager
                .model_supports_tools(&self.provider_name, &self.model)
                .unwrap_or(true)
        });
        let http_client = create_http_client(self.config_manager)?;
        let formatter = self
            .formatter
            .unwrap_or_else(|| Box::new(GenericPayloadFormatter));

        Ok(OpenAiCompatibleClient {
            base,
            api_url,
            api_key: self.api_key,
            http_client,
            formatter,
            supports_tools,
        })
    }
}

// ── Client impl ────────────────────────────────────────────────────────────

impl OpenAiCompatibleClient {
    pub fn builder(config_manager: &ConfigManager) -> OpenAiCompatibleClientBuilder<'_> {
        OpenAiCompatibleClientBuilder::new(config_manager)
    }

    /// Build the messages array for an OpenAI-compatible chat completion request.
    /// Delegates to the standalone `MessageBuilder` for testability.
    pub fn build_messages(&self, data: &[DataSource]) -> Vec<Value> {
        MessageBuilder {
            formatter: self.formatter.as_ref(),
            model: &self.base.state.model,
            system_prompt: self.base.state.get_effective_system_prompt(),
            conversation: &self.base.state.conversation,
            pending_data: data,
        }
        .build()
    }
}

// ── LlmClient impl ─────────────────────────────────────────────────────────

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
    fn should_send_pdf_as_base64(&self) -> bool {
        true
    }

    async fn send(
        &mut self,
        data: Vec<DataSource>,
        tool_schemas: Vec<Value>,
    ) -> anyhow::Result<crate::llm::models::LlmResponse> {
        let messages = self.build_messages(&data);
        let mut body = json!({
            "model": self.base.state.model,
            "messages": messages,
        });

        if self.supports_tools && !tool_schemas.is_empty() {
            body["tools"] = json!(
                tool_schemas
                    .iter()
                    .map(|s| json!({"type": "function", "function": s}))
                    .collect::<Vec<_>>()
            );
        }

        tracing::debug!(
            "LLM Request (to {}): {}",
            self.api_url,
            serde_json::to_string_pretty(&body).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "JSON serialization failed");
                String::new()
            })
        );

        let res = self
            .http_client
            .post(&self.api_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .context("Failed to send request to LLM API")?;

        let resp_json: Value = res
            .json()
            .await
            .context("Failed to parse LLM API response as JSON")?;

        tracing::debug!(
            "LLM Response: {}",
            serde_json::to_string_pretty(&resp_json).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "JSON serialization failed");
                String::new()
            })
        );

        if let Some(err) = resp_json.get("error") {
            return Err(anyhow::anyhow!("API Error: {}", err));
        }

        let choice = &resp_json["choices"][0];
        let msg = &choice["message"];

        let usage = resp_json
            .get("usage")
            .and_then(|u| serde_json::from_value::<crate::llm::models::Usage>(u.clone()).ok());

        // Delegate response parsing to the standalone module.
        let parsed = response_parser::parse_assistant_message(msg);

        let model_msg = Message {
            role: Role::Assistant,
            parts: parsed.message_parts,
        };
        self.update_history(&data, model_msg);

        Ok(crate::llm::models::LlmResponse {
            content: parsed.text,
            tool_name: None,
            usage,
        })
    }

    async fn send_as_verifier(
        &mut self,
        data: Vec<DataSource>,
        tool_schema: Value,
    ) -> anyhow::Result<Value> {
        let messages = self.build_messages(&data);
        let tool_name = tool_schema["name"]
            .as_str()
            .unwrap_or("submit_verdict")
            .to_string();
        let body = json!({
            "model": self.base.state.model,
            "messages": messages,
            "tools": [{"type": "function", "function": tool_schema}],
            "tool_choice": {"type": "function", "function": {"name": tool_name}}
        });

        tracing::debug!(
            "LLM Verifier Request (to {}): {}",
            self.api_url,
            serde_json::to_string_pretty(&body).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "JSON serialization failed");
                String::new()
            })
        );

        let res = self
            .http_client
            .post(&self.api_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .context("Failed to send request to LLM API")?;

        let resp_json: Value = res
            .json()
            .await
            .context("Failed to parse LLM API response as JSON")?;

        tracing::debug!(
            "LLM Verifier Response: {}",
            serde_json::to_string_pretty(&resp_json).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "JSON serialization failed");
                String::new()
            })
        );

        // 1. Check for API-level error field
        if let Some(err) = resp_json.get("error") {
            return Err(anyhow::anyhow!("API Error: {}", err));
        }

        // 2. Validate response structure
        let choice = match resp_json.get("choices").and_then(|c| c.get(0)) {
            Some(c) => c,
            None => {
                return Err(anyhow::anyhow!(
                    "Invalid response from LLM: no choices found. Full response: {}",
                    resp_json
                ));
            }
        };

        let msg = &choice["message"];

        // 3. Check for refusal (OpenAI safety filters)
        if let Some(refusal) = msg.get("refusal").and_then(|v| v.as_str()) {
            return Err(anyhow::anyhow!("Model refused to verify: {}", refusal));
        }

        // 4. Extract tool calls
        if let Some(tool_calls) = msg.get("tool_calls").and_then(|v| v.as_array())
            && !tool_calls.is_empty()
        {
            let tc = &tool_calls[0];
            let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
            return Ok(serde_json::from_str(args_str).unwrap_or(json!({})));
        }

        // 5. Fallback: If no tool call, check if it returned text (e.g. refused or explained)
        if let Some(content) = msg.get("content").and_then(|v| v.as_str()) {
            return Err(anyhow::anyhow!(
                "Verifier returned text instead of tool call: \"{}\". This usually means the model refused the request or is not capable of tool calling.",
                content
            ));
        }

        Err(anyhow::anyhow!(
            "Verifier did not return a tool call. Raw response: {}",
            resp_json
        ))
    }
}
