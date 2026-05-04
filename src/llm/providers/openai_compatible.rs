use crate::config::ConfigManager;
use crate::llm::base::{BaseLlmClientData, LlmClient, ProviderSpec, create_http_client};
use crate::llm::models::{ClientState, DataSource, Message, MessagePart, Role};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::HashMap;

/// Trait to handle provider-specific payload formatting.
/// This decouples the generic client from specific API quirks (OpenRouter, Anthropic, etc.)
pub trait PayloadFormatter: Send + Sync {
    fn format_text(&self, text: &str) -> Value {
        json!({"type": "text", "text": text})
    }
    fn format_image(&self, mime: &str, data: &str) -> Value {
        json!({
            "type": "image_url",
            "image_url": { "url": format!("data:{};base64,{}", mime, data) }
        })
    }
    fn format_pdf(&self, _data: &str, _filename: Option<&str>) -> Option<Value>;
    fn format_audio(&self, mime: &str, data: &str) -> Value {
        json!({
            "type": "input_audio",
            "input_audio": {
                "data": data,
                "format": mime.split('/').next_back().unwrap_or("mp3")
            }
        })
    }
}

pub struct GenericPayloadFormatter;
impl PayloadFormatter for GenericPayloadFormatter {
    fn format_pdf(&self, data: &str, _filename: Option<&str>) -> Option<Value> {
        // Default OpenAI compatibility: treat as image or ignore if not supported
        Some(json!({
            "type": "image_url",
            "image_url": { "url": format!("data:application/pdf;base64,{}", data) }
        }))
    }
}

pub struct HighFeaturePayloadFormatter {
    pub is_anthropic_gemini: bool,
}
impl PayloadFormatter for HighFeaturePayloadFormatter {
    fn format_pdf(&self, data: &str, _filename: Option<&str>) -> Option<Value> {
        if self.is_anthropic_gemini {
            // Anthropic/Gemini style native PDF support
            Some(json!({
                "type": "document",
                "source": { "type": "base64", "media_type": "application/pdf", "data": data }
            }))
        } else {
            // Default to image_url fallback
            Some(json!({
                "type": "image_url",
                "image_url": { "url": format!("data:application/pdf;base64,{}", data) }
            }))
        }
    }
}

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

        let mut supports_tools = true;
        let config = self.config_manager.get_config()?;
        let model_id_lower = self.model.to_lowercase();
        for rule in &config.rules {
            if let Ok(re) = regex::Regex::new(&rule.pattern)
                && re.is_match(&model_id_lower)
            {
                supports_tools = rule.supports_tools;
            }
        }

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

impl OpenAiCompatibleClient {
    pub fn builder(config_manager: &ConfigManager) -> OpenAiCompatibleClientBuilder<'_> {
        OpenAiCompatibleClientBuilder::new(config_manager)
    }

    pub fn build_messages(&self, data: &[DataSource]) -> Vec<Value> {
        let mut messages = Vec::new();

        if let Some(sp) = self.base.state.get_effective_system_prompt() {
            messages.push(json!({"role": "system", "content": sp}));
        }

        for m in &self.base.state.conversation {
            match m.role {
                Role::System => {
                    messages.push(json!({
                        "role": "system",
                        "content": m.get_text(true)
                    }));
                }
                Role::Tool => {
                    for part in &m.parts {
                        if let MessagePart::Part(cp) = part
                            && let Some(fr) = &cp.function_response
                        {
                            messages.push(json!({
                                "role": "tool",
                                "tool_call_id": fr.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                                "name": fr.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                                "content": fr.get("response").cloned().unwrap_or(json!("")).to_string()
                            }));
                        }
                    }
                }
                _ => {
                    let role = if m.role == Role::Assistant || m.role == Role::Model {
                        "assistant"
                    } else {
                        "user"
                    };
                    let mut parts = Vec::new();
                    let mut tool_calls = Vec::new();

                    for part in &m.parts {
                        match part {
                            MessagePart::Text(t) => parts.push(self.formatter.format_text(t)),
                            MessagePart::Part(cp) => {
                                if let Some(t) = &cp.text {
                                    parts.push(self.formatter.format_text(t));
                                }
                                if let Some(fc) = &cp.function_call {
                                    tool_calls.push(json!({
                                        "id": fc.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                                        "type": "function",
                                        "function": {
                                            "name": fc.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                                            "arguments": fc.get("arguments").cloned().unwrap_or(json!({})).to_string()
                                        }
                                    }));
                                }
                                if let Some(id) = &cp.inline_data {
                                    let mime =
                                        id.get("mimeType").and_then(|v| v.as_str()).unwrap_or("");
                                    let b64 = id.get("data").and_then(|v| v.as_str()).unwrap_or("");
                                    if mime == "application/pdf" {
                                        if let Some(v) = self.formatter.format_pdf(
                                            b64,
                                            id.get("filename").and_then(|v| v.as_str()),
                                        ) {
                                            parts.push(v);
                                        }
                                    } else if mime.starts_with("image/") {
                                        parts.push(self.formatter.format_image(mime, b64));
                                    } else if mime.starts_with("audio/") {
                                        parts.push(self.formatter.format_audio(mime, b64));
                                    }
                                }
                            }
                        }
                    }
                    let mut msg = json!({"role": role, "content": if parts.len() == 1 && parts[0]["type"] == "text" { parts[0]["text"].clone() } else { Value::Array(parts) }});
                    if !tool_calls.is_empty() {
                        msg["tool_calls"] = Value::Array(tool_calls);
                    }
                    messages.push(msg);
                }
            }
        }

        // Current pending data
        let mut current_parts = Vec::new();
        for d in data {
            if d.content_type == "text/plain" {
                current_parts.push(self.formatter.format_text(d.content.as_str().unwrap_or("")));
            } else if d.content_type == "application/pdf" {
                if let Some(v) = self.formatter.format_pdf(
                    d.content.as_str().unwrap_or(""),
                    d.metadata.get("filename").and_then(|v| v.as_str()),
                ) {
                    current_parts.push(v);
                }
            } else if d.content_type.starts_with("image/") {
                current_parts.push(
                    self.formatter
                        .format_image(&d.content_type, d.content.as_str().unwrap_or("")),
                );
            }
        }
        if !current_parts.is_empty() {
            messages.push(json!({"role": "user", "content": if current_parts.len() == 1 && current_parts[0]["type"] == "text" { current_parts[0]["text"].clone() } else { Value::Array(current_parts) }}));
        }

        messages
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
    fn should_send_pdf_as_base64(&self) -> bool {
        true
    }

    async fn send(
        &mut self,
        data: Vec<DataSource>,
        tool_schemas: Vec<Value>,
    ) -> anyhow::Result<(Option<String>, Option<String>)> {
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

        let res = self
            .http_client
            .post(&self.api_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;

        let resp_json: Value = res.json().await?;
        if let Some(err) = resp_json.get("error") {
            return Err(anyhow::anyhow!("API Error: {}", err));
        }

        let choice = &resp_json["choices"][0];
        let msg = &choice["message"];
        let text = msg["content"].as_str().map(|s| s.to_string());

        let mut message_parts = Vec::new();
        if let Some(t) = &text {
            message_parts.push(MessagePart::Text(t.clone()));
        }

        if let Some(tool_calls) = msg.get("tool_calls").and_then(|v| v.as_array()) {
            for tc in tool_calls {
                let mut fc = HashMap::new();
                fc.insert("id".to_string(), tc["id"].clone());
                fc.insert("name".to_string(), tc["function"]["name"].clone());
                let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
                let args: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
                fc.insert("arguments".to_string(), args);
                message_parts.push(MessagePart::Part(Box::new(
                    crate::llm::models::ContentPart {
                        function_call: Some(fc),
                        ..Default::default()
                    },
                )));
            }
        }

        let model_msg = Message {
            role: Role::Assistant,
            parts: message_parts,
        };
        self.update_history(&data, model_msg);

        Ok((text, None))
    }

    async fn send_as_verifier(
        &mut self,
        data: Vec<DataSource>,
        tool_schema: Value,
    ) -> anyhow::Result<Value> {
        let messages = self.build_messages(&data);
        let body = json!({
            "model": self.base.state.model,
            "messages": messages,
            "tools": [{"type": "function", "function": tool_schema}],
            "tool_choice": {"type": "function", "function": {"name": tool_schema["name"]}}
        });

        let res = self
            .http_client
            .post(&self.api_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;

        let resp_json: Value = res.json().await?;
        let choice = &resp_json["choices"][0];
        let msg = &choice["message"];

        if let Some(tool_calls) = msg.get("tool_calls").and_then(|v| v.as_array()) {
            let tc = &tool_calls[0];
            let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
            return Ok(serde_json::from_str(args_str).unwrap_or(json!({})));
        }

        Err(anyhow::anyhow!("Verifier did not return a tool call"))
    }
}
