use crate::config::ConfigManager;
use crate::llm::base::{BaseLlmClientData, LlmClient, ProviderSpec, create_http_client};
use crate::llm::models::{ClientState, ContentPart, DataSource, Message, MessagePart, Role};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::HashMap;

/// Generic OpenAI-compatible API client.
/// Supports any provider that follows the OpenAI Chat Completions API format.
pub struct OpenAiCompatibleClient {
    pub base: BaseLlmClientData,
    pub api_url: String,
    pub api_key: String,
    pub http_client: reqwest::Client,
    pub supports_tools: bool,
    pub supports_vision: bool,
    pub image_generation_enabled: bool,
    pub video_generation_enabled: bool,
    pub audio_generation_enabled: bool,
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
    ) -> anyhow::Result<Self> {
        let spec = ProviderSpec {
            api_key_name: "api_key".to_string(),
            config_section: provider_name.to_string(),
            pdf_as_base64: true,
        };
        let _model_config = config_manager.get_model_config(provider_name, model);
        let config = config_manager.get_config()?;
        let base = BaseLlmClientData::new(config_manager, model, spec, stdout, raw);

        let api_url = if api_url.ends_with("/chat/completions") {
            api_url.to_string()
        } else {
            format!("{}/chat/completions", api_url.trim_end_matches('/'))
        };

        let mut supports_tools = true;
        let mut image_generation_enabled = false;
        let mut video_generation_enabled = false;
        let mut audio_generation_enabled = false;

        // Apply dynamic rules based on model ID
        let model_id_lower = model.to_lowercase();
        for rule in &config.rules {
            if let Ok(re) = regex::Regex::new(&rule.pattern)
                && re.is_match(&model_id_lower)
            {
                supports_tools = rule.supports_tools;
                image_generation_enabled = rule.image_generation;
                video_generation_enabled = rule.video_generation;
                audio_generation_enabled = rule.audio_generation;
            }
        }

        let http_client = create_http_client(config_manager)?;

        Ok(Self {
            base,
            api_url,
            api_key: api_key.to_string(),
            http_client,
            supports_tools,
            supports_vision: true,
            image_generation_enabled,
            video_generation_enabled,
            audio_generation_enabled,
        })
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
                                } else if mime.starts_with("video/") {
                                    content_parts.push(json!({
                                        "type": "video_url",
                                        "video_url": { "url": Self::data_url(mime, data) }
                                    }));
                                } else {
                                    // Fallback for others
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
                    t == "image_url" || t == "input_audio" || t == "video_url"
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
                ct if ct == "application/pdf" || ct.starts_with("image/") => {
                    if let Some(b64) = d.content.as_str() {
                        new_parts.push(json!({
                            "type": "image_url",
                            "image_url": { "url": Self::data_url(ct, b64) }
                        }));
                    }
                }
                ct if ct.starts_with("video/") => {
                    if let Some(b64) = d.content.as_str() {
                        new_parts.push(json!({
                            "type": "video_url",
                            "video_url": { "url": Self::data_url(ct, b64) }
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
                t == "image_url" || t == "input_audio" || t == "video_url"
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

        // For OpenRouter, we need to handle specific modalities with separate endpoints
        let is_openrouter = self.base.config_section == "openrouter";
        let mut request_url = self.api_url.clone();

        // If the model is exclusively a video or audio processing model, we override the endpoint
        if is_openrouter && self.video_generation_enabled {
            request_url = request_url.replace("/chat/completions", "/videos");
        } else if is_openrouter && self.audio_generation_enabled {
            request_url = request_url.replace("/chat/completions", "/audio/speech");
        }

        let body = if is_openrouter && self.video_generation_enabled {
            // Build OpenRouter /api/v1/videos request
            let prompt = messages
                .iter()
                .filter_map(|m| {
                    if m.get("role").and_then(|v| v.as_str()) == Some("user") {
                        let content = m.get("content")?;
                        if let Some(s) = content.as_str() {
                            Some(s.to_string())
                        } else if let Some(arr) = content.as_array() {
                            let mut texts = Vec::new();
                            for p in arr {
                                if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                                    texts.push(t.to_string());
                                }
                            }
                            if texts.is_empty() {
                                None
                            } else {
                                Some(texts.join("\n"))
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");

            let mut req = json!({
                "model": self.base.state.model,
                "prompt": prompt,
            });

            // Extract frame images or input references from user messages if they exist
            let mut images = Vec::new();
            for m in &messages {
                if let Some(content) = m.get("content").and_then(|v| v.as_array()) {
                    for p in content {
                        if p.get("type").and_then(|v| v.as_str()) == Some("image_url")
                            && let Some(image_url) = p.get("image_url") {
                                images.push(json!({
                                    "type": "image_url",
                                    "image_url": image_url.clone()
                                }));
                            }
                    }
                }
            }

            if !images.is_empty() {
                // By default put them as input_references. You could refine this to frame_images if needed.
                req["input_references"] = json!(images);
            }

            req
        } else if is_openrouter && self.audio_generation_enabled {
            // Build OpenRouter /api/v1/audio/speech request
            let input = messages
                .iter()
                .filter_map(|m| {
                    if m.get("role").and_then(|v| v.as_str()) == Some("user") {
                        let content = m.get("content")?;
                        if let Some(s) = content.as_str() {
                            return Some(s.to_string());
                        } else if let Some(arr) = content.as_array() {
                            let mut texts = Vec::new();
                            for p in arr {
                                if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                                    texts.push(t.to_string());
                                }
                            }
                            if !texts.is_empty() {
                                return Some(texts.join("\n"));
                            }
                        }
                    }
                    None
                })
                .collect::<Vec<_>>()
                .join("\n");

            json!({
                "model": self.base.state.model,
                "input": input,
                // "voice": ... typically required but depends on provider. "alloy" works for OpenAI TTS.
                // You can add logic to extract voice from settings or prompt if needed.
                "voice": "alloy",
            })
        } else {
            // Standard /chat/completions payload
            let mut req = json!({
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
                req["tools"] = json!(tools);
                req["tool_choice"] = json!("auto");
            }

            if is_openrouter && self.image_generation_enabled {
                req["modalities"] = json!(["image"]);
            }
            req
        };

        log::debug!(
            "API Request: URL: {}, Body: {}",
            request_url,
            serde_json::to_string(&body).unwrap_or_default()
        );

        let res = self
            .http_client
            .post(&request_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = res.status();

        // Check if response is JSON or BINARY (like audio)
        let is_json = res
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.contains("application/json"))
            .unwrap_or(false);

        if !is_json && is_openrouter && self.audio_generation_enabled {
            let bytes = res.bytes().await?;
            use base64::{Engine as _, engine::general_purpose};
            let b64 = general_purpose::STANDARD.encode(&bytes);
            let mime_type = "audio/mpeg"; // default MP3

            let mut inline_data = HashMap::new();
            inline_data.insert("mimeType".to_string(), json!(mime_type));
            inline_data.insert("data".to_string(), json!(b64));

            let audio_part = MessagePart::Part(Box::new(ContentPart {
                inline_data: Some(inline_data),
                ..Default::default()
            }));

            let text = "Audio generated successfully.".to_string();
            let model_msg = Message {
                role: Role::Assistant,
                parts: vec![MessagePart::Text(text.clone()), audio_part],
            };
            self.update_history(&data, model_msg);
            return Ok((Some(text), None));
        }

        let resp: Value = res.json().await?;
        log::debug!(
            "API Response: Status: {}, Body: {}",
            status,
            serde_json::to_string(&resp).unwrap_or_default()
        );

        if let Some(error) = resp.get("error") {
            return Err(anyhow::anyhow!("API Error: {}", error));
        }

        // OpenRouter /api/v1/videos response returns job ID and polling URL
        if is_openrouter && self.video_generation_enabled
            && let Some(job_id) = resp.get("id").and_then(|v| v.as_str()) {
                let mut text = format!("Video generation submitted. Job ID: {}\n", job_id);
                if let Some(polling_url) = resp.get("polling_url").and_then(|v| v.as_str()) {
                    text.push_str(&format!("To check status: GET {}\n", polling_url));
                    text.push_str("(Currently, the CLI does not automatically block and poll for video jobs due to high completion times.)");
                }
                let model_msg = Message {
                    role: Role::Assistant,
                    parts: vec![MessagePart::Text(text.clone())],
                };
                self.update_history(&data, model_msg);
                return Ok((Some(text), None));
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
                    if let Some(video_url) = part
                        .get("video_url")
                        .and_then(|v| v.get("url"))
                        .and_then(|v| v.as_str())
                        && video_url.starts_with("data:")
                        && let Some(comma_pos) = video_url.find(',')
                    {
                        let header = &video_url[..comma_pos];
                        let b64_data = &video_url[comma_pos + 1..];
                        let mime_type = header
                            .trim_start_matches("data:")
                            .split(';')
                            .next()
                            .unwrap_or("video/mp4");

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

        // Handle possible videos array
        if let Some(videos) = message.get("videos").and_then(|v| v.as_array()) {
            for video in videos {
                if let Some(video_url) = video
                    .get("video_url")
                    .or(video.get("videoUrl"))
                    .and_then(|v| v.get("url"))
                    .and_then(|v| v.as_str())
                    && video_url.starts_with("data:")
                    && let Some(comma_pos) = video_url.find(',')
                {
                    let header = &video_url[..comma_pos];
                    let b64_data = &video_url[comma_pos + 1..];
                    let mime_type = header
                        .trim_start_matches("data:")
                        .split(';')
                        .next()
                        .unwrap_or("video/mp4");

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

        log::debug!(
            "Verifier API Request: URL: {}, Body: {}",
            self.api_url,
            serde_json::to_string(&body).unwrap_or_default()
        );

        let res = self
            .http_client
            .post(&self.api_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;

        let status = res.status();
        let resp: Value = res.json().await?;
        log::debug!(
            "Verifier API Response: Status: {}, Body: {}",
            status,
            serde_json::to_string(&resp).unwrap_or_default()
        );

        let content = resp["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("{}");
        match serde_json::from_str(content) {
            Ok(v) => Ok(v),
            Err(_) => Ok(json!({"safe": false, "reason": content})),
        }
    }
}
