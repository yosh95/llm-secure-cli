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

        // 1. Mandatory System Prompt (with Date)
        if let Some(sp) = self.base.state.get_effective_system_prompt() {
            messages.push(json!({"role": "system", "content": sp}));
        }

        let mut processed_messages = Vec::new();
        // Keep track of Tool IDs generated by the assistant to validate subsequent Tool responses.
        let mut available_tool_calls = std::collections::HashSet::new();

        for m in &self.base.state.conversation {
            match m.role {
                Role::System => {
                    let text = m.get_text(true);
                    if !text.is_empty() {
                        processed_messages.push(json!({
                            "role": "system",
                            "content": text
                        }));
                    }
                }
                Role::Tool => {
                    let mut tool_contents = Vec::new();
                    for part in &m.parts {
                        if let MessagePart::Part(cp) = part
                            && let Some(fr) = &cp.function_response
                        {
                            let content = match fr.get("response") {
                                Some(Value::String(s)) => s.clone(),
                                Some(v) => v.to_string(),
                                None => "".to_string(),
                            };
                            let id = fr.get("id").and_then(|v| v.as_str()).unwrap_or("");
                            let tool_name =
                                fr.get("name").and_then(|v| v.as_str()).unwrap_or("tool");

                            if !id.is_empty() && available_tool_calls.contains(id) {
                                // Anthropic/OpenRouter/Bedrock style: specific tool_call_id in a role:tool message.
                                // Note: We still push each one as a separate message for OpenAI compatibility,
                                // BUT we ensure that NO 'user' role messages are interleaved if we can help it.
                                processed_messages.push(json!({
                                    "role": "tool",
                                    "tool_call_id": id,
                                    "name": tool_name,
                                    "content": content
                                }));
                            } else {
                                // Ororphaned or fallback
                                tool_contents
                                    .push(format!("Tool Result ({}): {}", tool_name, content));
                            }
                        }
                    }
                    if !tool_contents.is_empty() {
                        processed_messages.push(json!({
                            "role": "user",
                            "content": tool_contents.join("\n\n")
                        }));
                    }
                }
                _ => {
                    let role = if m.role == Role::Assistant || m.role == Role::Model {
                        "assistant"
                    } else {
                        "user"
                    };

                    // If this is an assistant message, it defines the set of valid tool_call_ids
                    // that the *next* tool messages can use.
                    if role == "assistant" {
                        available_tool_calls.clear();
                    }

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
                                    let id = fc.get("id").and_then(|v| v.as_str()).unwrap_or("");
                                    if role == "assistant" && !id.is_empty() {
                                        available_tool_calls.insert(id.to_string());
                                    }

                                    tool_calls.push(json!({
                                        "id": id,
                                        "type": "function",
                                        "function": {
                                            "name": fc.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                                            "arguments": fc.get("arguments").cloned().unwrap_or(json!({})).to_string()
                                        }
                                    }));
                                }
                                // ... (PDF/Image/Audio logic remains same)
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
                    let content_value = if parts.is_empty() {
                        Value::String("".to_string())
                    } else if parts.len() == 1 && parts[0]["type"] == "text" {
                        parts[0]["text"].clone()
                    } else {
                        Value::Array(parts)
                    };

                    let mut msg = json!({
                        "role": role,
                        "content": content_value,
                    });

                    if !tool_calls.is_empty() {
                        msg["tool_calls"] = Value::Array(tool_calls);
                        // Some providers (like Amazon Bedrock via OpenRouter/Nova) fail if 'content' is an empty string
                        // when 'tool_calls' is present.
                        if role == "assistant"
                            && (content_value == Value::String("".to_string())
                                || content_value == Value::Array(vec![]))
                            && let Some(obj) = msg.as_object_mut()
                        {
                            obj.remove("content");
                        }
                    }
                    processed_messages.push(msg);
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
            processed_messages.push(json!({"role": "user", "content": if current_parts.len() == 1 && current_parts[0]["type"] == "text" { current_parts[0]["text"].clone() } else { Value::Array(current_parts) }}));
        }

        messages.extend(processed_messages);
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
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );

        let res = self
            .http_client
            .post(&self.api_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;

        let resp_json: Value = res.json().await?;

        tracing::debug!(
            "LLM Response: {}",
            serde_json::to_string_pretty(&resp_json).unwrap_or_default()
        );

        if let Some(err) = resp_json.get("error") {
            return Err(anyhow::anyhow!("API Error: {}", err));
        }

        // Report if the model changed (e.g. via OpenRouter fallback)
        let mut redirect_msg = None;
        if let Some(resp_model) = resp_json.get("model").and_then(|v| v.as_str())
            && resp_model != self.base.state.model
        {
            redirect_msg = Some(format!(
                "Model redirected from '{}' to '{}'",
                self.base.state.model, resp_model
            ));
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

        Ok(crate::llm::models::LlmResponse {
            content: text,
            tool_name: None,
            tool_args: redirect_msg,
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
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );

        let res = self
            .http_client
            .post(&self.api_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;

        let resp_json: Value = res.json().await?;

        tracing::debug!(
            "LLM Verifier Response: {}",
            serde_json::to_string_pretty(&resp_json).unwrap_or_default()
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

        // 4. Fallback: If no tool call, check if it returned text (e.g. refused or explained)
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
