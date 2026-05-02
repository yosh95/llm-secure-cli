use crate::config::ConfigManager;
use crate::llm::base::{BaseLlmClientData, LlmClient, ProviderSpec};
use crate::llm::models::{ClientState, ContentPart, DataSource, Message, MessagePart, Role};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::HashMap;

use once_cell::sync::Lazy;

static CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .build()
        .expect("Failed to create reqwest client")
});

/// Generic OpenAI-compatible API client.
/// Supports any provider that follows the OpenAI Chat Completions API format.
pub struct OpenAiCompatibleClient {
    pub base: BaseLlmClientData,
    pub api_url: String,
    pub api_key: String,
    pub supports_tools: bool,
    pub supports_vision: bool,
    pub image_generation_enabled: bool,
}

impl OpenAiCompatibleClient {
    pub fn new(
        config_manager: &ConfigManager,
        provider_name: &str,
        api_url: &str,
        api_key: &str,
        model: &str,
        stdout: bool,
        raw: bool,
    ) -> Self {
        let spec = ProviderSpec {
            api_key_name: "api_key".to_string(),
            config_section: provider_name.to_string(),
            pdf_as_base64: true,
        };
        let model_config = config_manager.get_model_config(provider_name, model);
        let base = BaseLlmClientData::new(config_manager, model, spec, stdout, raw);

        let api_url = if api_url.ends_with("/chat/completions") {
            api_url.to_string()
        } else {
            format!("{}/chat/completions", api_url.trim_end_matches('/'))
        };

        let image_generation_enabled = model_config
            .get("image_generation")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let supports_tools = model_config
            .get("tools")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        Self {
            base,
            api_url,
            api_key: api_key.to_string(),
            supports_tools,
            supports_vision: true,
            image_generation_enabled,
        }
    }

    fn data_url(mime_type: &str, b64_data: &str) -> String {
        format!("data:{};base64,{}", mime_type, b64_data)
    }

    fn build_messages(&self, data: &[DataSource]) -> Vec<Value> {
        let mut messages = Vec::new();

        // Add system prompt if present
        if let Some(sp) = &self.base.state.system_prompt {
            messages.push(json!({"role": "system", "content": sp}));
        }

        // Convert conversation history
        for m in &self.base.state.conversation {
            if m.role == Role::System {
                continue;
            }

            if m.role == Role::Tool {
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
                        if let Some(t) = &cp.thought {
                            content_parts
                                .push(json!({"type": "text", "text": format!("[Thinking] {}", t)}));
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
                            let mime = id
                                .get("mimeType")
                                .or(id.get("mime_type"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let data = id.get("data").and_then(|v| v.as_str()).unwrap_or("");
                            if !mime.is_empty() && !data.is_empty() {
                                if mime == "application/pdf" || mime.starts_with("image/") {
                                    content_parts.push(json!({
                                        "type": "image_url",
                                        "image_url": { "url": Self::data_url(mime, data) }
                                    }));
                                } else if mime.starts_with("audio/") {
                                    content_parts.push(json!({
                                        "type": "input_audio",
                                        "input_audio": {
                                            "data": data,
                                            "format": mime.split('/').next_back().unwrap_or("mp3")
                                        }
                                    }));
                                } else {
                                    // Fallback for video etc.
                                    content_parts.push(json!({
                                        "type": "image_url",
                                        "image_url": { "url": Self::data_url(mime, data) }
                                    }));
                                }
                            }
                        }
                    }
                }
            }

            if !content_parts.is_empty() {
                let tool_calls: Vec<Value> = content_parts
                    .iter()
                    .filter_map(|p| p.get("tool_call").cloned())
                    .collect();

                // Exclude tool_call type parts from content — those belong in tool_calls only
                let content_only_parts: Vec<Value> = content_parts
                    .into_iter()
                    .filter(|p| p.get("type").and_then(|v| v.as_str()) != Some("tool_call"))
                    .collect();

                let has_media = content_only_parts.iter().any(|p| {
                    let t = p.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    t == "image_url" || t == "input_audio"
                });

                let content = if content_only_parts.is_empty() {
                    // No text/media — content must be null for assistant tool-call messages
                    Value::Null
                } else if has_media || content_only_parts.len() > 1 {
                    Value::Array(content_only_parts)
                } else {
                    let text = content_only_parts[0]
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    Value::String(text)
                };

                let mut msg = json!({"role": role, "content": content});
                if role == "assistant" && !tool_calls.is_empty() {
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
                ct if ct == "application/pdf"
                    || ct.starts_with("image/")
                    || ct.starts_with("video/") =>
                {
                    if let Some(b64) = d.content.as_str() {
                        new_parts.push(json!({
                            "type": "image_url",
                            "image_url": { "url": Self::data_url(ct, b64) }
                        }));
                    }
                }
                ct if ct.starts_with("audio/") => {
                    if let Some(b64) = d.content.as_str() {
                        new_parts.push(json!({
                            "type": "input_audio",
                            "input_audio": {
                                "data": b64,
                                "format": ct.split('/').next_back().unwrap_or("mp3")
                            }
                        }));
                    }
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
            let has_media = new_parts.iter().any(|p| {
                let t = p.get("type").and_then(|v| v.as_str()).unwrap_or("");
                t == "image_url" || t == "input_audio"
            });
            let content = if has_media || new_parts.len() > 1 {
                Value::Array(new_parts)
            } else {
                new_parts[0]
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .into()
            };
            messages.push(json!({"role": "user", "content": content}));
        }

        messages
    }

    fn build_tool_schemas(&self, tool_schemas: Vec<Value>) -> Vec<Value> {
        if self.supports_tools {
            tool_schemas
                .into_iter()
                .map(|s| {
                    json!({
                        "type": "function",
                        "function": s
                    })
                })
                .collect()
        } else {
            Vec::new()
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

        let mut tools = if self.supports_tools && self.base.state.tools_enabled {
            self.build_tool_schemas(tool_schemas)
        } else {
            Vec::new()
        };
        if self.supports_tools && self.image_generation_enabled {
            tools.push(json!({ "type": "image_generation" }));
        }
        if !tools.is_empty() {
            body["tools"] = json!(tools);
            body["tool_choice"] = json!("auto");
        }

        if self.base.config_section == "openrouter" && self.image_generation_enabled {
            body["modalities"] = json!(["text", "image"]);
        }

        let res = CLIENT
            .post(&self.api_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let resp: Value = res.json().await?;
        if let Some(error) = resp.get("error") {
            return Err(anyhow::anyhow!("API Error: {}", error));
        }

        let choice = resp["choices"][0].clone();
        let message = &choice["message"];

        let mut assistant_parts = Vec::new();
        let mut text = None;

        if let Some(content) = message.get("content") {
            if let Some(s) = content.as_str() {
                text = Some(s.to_string());
                assistant_parts.push(MessagePart::Text(s.to_string()));
            } else if let Some(arr) = content.as_array() {
                let mut full_text = String::new();
                for part in arr {
                    if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                        full_text.push_str(t);
                        assistant_parts.push(MessagePart::Text(t.to_string()));
                    }
                    if let Some(image_url) = part
                        .get("image_url")
                        .and_then(|v| v.get("url"))
                        .and_then(|v| v.as_str())
                        && image_url.starts_with("data:")
                        && let Some(comma_pos) = image_url.find(',')
                    {
                        let header = &image_url[..comma_pos];
                        let b64_data = &image_url[comma_pos + 1..];
                        let mime_type = header
                            .trim_start_matches("data:")
                            .split(';')
                            .next()
                            .unwrap_or("image/png");

                        let mut inline_data = HashMap::new();
                        inline_data.insert("mimeType".to_string(), json!(mime_type));
                        inline_data.insert("data".to_string(), json!(b64_data));

                        assistant_parts.push(MessagePart::Part(Box::new(ContentPart {
                            inline_data: Some(inline_data),
                            ..Default::default()
                        })));
                    }
                    if let Some(id) = part.get("inline_data") {
                        let mut inline_data = HashMap::new();
                        if let Some(m) = id
                            .get("mimeType")
                            .or(id.get("mime_type"))
                            .and_then(|v| v.as_str())
                        {
                            inline_data.insert("mimeType".to_string(), json!(m));
                        }
                        if let Some(d) = id.get("data").and_then(|v| v.as_str()) {
                            inline_data.insert("data".to_string(), json!(d));
                        }
                        if !inline_data.is_empty() {
                            assistant_parts.push(MessagePart::Part(Box::new(ContentPart {
                                inline_data: Some(inline_data),
                                ..Default::default()
                            })));
                        }
                    }
                }
                if !full_text.is_empty() {
                    text = Some(full_text);
                }
            }
        }

        // Handle OpenRouter-specific images array
        if let Some(images) = message.get("images").and_then(|v| v.as_array()) {
            for image in images {
                if let Some(image_url) = image
                    .get("image_url")
                    .or(image.get("imageUrl"))
                    .and_then(|v| v.get("url"))
                    .and_then(|v| v.as_str())
                    && image_url.starts_with("data:")
                    && let Some(comma_pos) = image_url.find(',')
                {
                    let header = &image_url[..comma_pos];
                    let b64_data = &image_url[comma_pos + 1..];
                    let mime_type = header
                        .trim_start_matches("data:")
                        .split(';')
                        .next()
                        .unwrap_or("image/png");

                    let mut inline_data = HashMap::new();
                    inline_data.insert("mimeType".to_string(), json!(mime_type));
                    inline_data.insert("data".to_string(), json!(b64_data));

                    assistant_parts.push(MessagePart::Part(Box::new(ContentPart {
                        inline_data: Some(inline_data),
                        ..Default::default()
                    })));
                }
            }
        }

        if let Some(citations) = resp.get("citations").and_then(|v| v.as_array()) {
            let mut citation_links = Vec::new();
            for (i, citation) in citations.iter().enumerate() {
                if let Some(url) = citation.as_str() {
                    citation_links.push(format!("{}. [Source {}]({})", i + 1, i + 1, url));
                }
            }
            if !citation_links.is_empty() {
                let citations_text = format!("\n\n**Sources:**\n{}", citation_links.join("\n"));
                if let Some(ref mut t) = text {
                    t.push_str(&citations_text);
                    if let Some(MessagePart::Text(last_t)) = assistant_parts
                        .iter_mut()
                        .rev()
                        .find(|p| matches!(p, MessagePart::Text(_)))
                    {
                        *last_t = t.clone();
                    } else {
                        assistant_parts.push(MessagePart::Text(citations_text));
                    }
                } else {
                    text = Some(citations_text.clone());
                    assistant_parts.push(MessagePart::Text(citations_text));
                }
            }
        }

        if let Some(tool_calls) = message.get("tool_calls").and_then(|v| v.as_array()) {
            for tc in tool_calls {
                if tc.get("type").and_then(|v| v.as_str()) == Some("function") {
                    let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let func = &tc["function"];
                    let name = func.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let args: Value = serde_json::from_str(
                        func.get("arguments")
                            .and_then(|v| v.as_str())
                            .unwrap_or("{}"),
                    )
                    .unwrap_or(json!({}));
                    let mut fc = HashMap::new();
                    fc.insert("id".to_string(), json!(id));
                    fc.insert("name".to_string(), json!(name));
                    fc.insert("arguments".to_string(), args);
                    assistant_parts.push(MessagePart::Part(Box::new(ContentPart {
                        function_call: Some(fc),
                        ..Default::default()
                    })));
                }
            }
        }

        let model_msg = Message {
            role: Role::Assistant,
            parts: assistant_parts,
        };
        self.update_history(&data, model_msg);
        Ok((text, None))
    }

    async fn send_as_verifier(
        &mut self,
        data: Vec<DataSource>,
        _tool_schema: Value,
    ) -> anyhow::Result<Value> {
        let messages = self.build_messages(&data);
        let body =
            json!({ "model": self.base.state.model, "messages": messages, "max_tokens": 1024 });
        let res = CLIENT
            .post(&self.api_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;
        let resp: Value = res.json().await?;
        let content = resp["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("{}");
        match serde_json::from_str(content) {
            Ok(v) => Ok(v),
            Err(_) => Ok(json!({"safe": false, "reason": content})),
        }
    }
}
