//! DeepInfra provider client.
//!
//! DeepInfra uses different API endpoints depending on the model modality:
//! - Chat/text generation: `POST /v1/openai/chat/completions` (OpenAI compatible)
//! - Image generation:     `POST /v1/inference/{model_id}`
//! - Video generation:     `POST /v1/inference/{model_id}`
//! - Audio generation:     `POST /v1/inference/{model_id}`
//!
//! This client automatically detects the model type from the cache and routes
//! requests to the appropriate endpoint with the correct payload format.

use crate::config::ConfigManager;
use crate::llm::base::{BaseLlmClientData, LlmClient, ProviderSpec, create_http_client};
use crate::llm::models::{ClientState, DataSource, LlmResponse, Message, MessagePart, Role, Usage};
use crate::llm::providers::message_builder::MessageBuilder;
use crate::llm::providers::openai_compatible::PayloadFormatter;
use crate::llm::providers::response_parser;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::HashMap;

/// Strip `data:[mime];base64,` prefix from a data URI if present.
/// Returns the raw base64 payload; if the input is not a data URI, returns it unchanged.
fn strip_data_uri_prefix(s: &str) -> &str {
    s.strip_prefix("data:")
        .and_then(|s| s.split_once(','))
        .map(|(_, data)| data)
        .unwrap_or(s)
}

/// DeepInfra model modality type.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DeepInfraModelType {
    /// Text generation / chat (OpenAI-compatible /chat/completions endpoint)
    Chat,
    /// Image generation (e.g. Stable Diffusion, SDXL)
    Image,
    /// Video generation
    Video,
    /// Audio generation / TTS / speech recognition
    Audio,
}

impl DeepInfraModelType {
    /// Determine model type from cache metadata, falling back to model name heuristics.
    pub fn from_model_id(model_id: &str, cache_model_type: Option<&str>) -> Self {
        // Use cache info if available
        if let Some(t) = cache_model_type {
            return match t {
                "image" => Self::Image,
                "video" => Self::Video,
                "audio" => Self::Audio,
                _ => Self::Chat,
            };
        }
        // Fallback: heuristic based on model name
        let lower = model_id.to_lowercase();
        if lower.contains("stable-diffusion")
            || lower.contains("sdxl")
            || lower.contains("dreamshaper")
            || lower.contains("realistic-vision")
            || lower.contains("playground-v2")
            || lower.contains("kandinsky")
            || lower.contains("dall-e")
            || lower.contains("flux")
            || lower.contains("midjourney")
        {
            Self::Image
        } else if lower.contains("text-to-video")
            || lower.contains("video")
            || (lower.contains("gen") && lower.contains("video"))
        {
            Self::Video
        } else if lower.contains("tts")
            || lower.contains("speech")
            || lower.contains("whisper")
            || lower.contains("vits")
        {
            Self::Audio
        } else {
            Self::Chat
        }
    }

    /// Returns the API path for this model type given the base URL and model ID.
    pub fn api_path(&self, base_url: &str, model: &str) -> String {
        match self {
            Self::Chat => {
                let base = base_url.trim_end_matches('/');
                if base.ends_with("/chat/completions") {
                    base.to_string()
                } else if base.contains("/openai") {
                    format!("{}/chat/completions", base)
                } else {
                    format!("{}/openai/chat/completions", base)
                }
            }
            // Image, Video, Audio all use /v1/inference/{model_id}
            Self::Image | Self::Video | Self::Audio => {
                let base = base_url.trim_end_matches('/');
                // Strip trailing /openai if present to get to /v1
                let v1_base = base.strip_suffix("/openai").unwrap_or(base);
                format!("{}/inference/{}", v1_base, model)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// DeepInfra Payload Formatter
// ---------------------------------------------------------------------------

pub struct DeepInfraFormatter;
impl PayloadFormatter for DeepInfraFormatter {
    fn format_pdf(&self, data: &str, _filename: Option<&str>) -> Option<Value> {
        Some(json!({
            "type": "image_url",
            "image_url": { "url": format!("data:application/pdf;base64,{}", data) }
        }))
    }
}

// ---------------------------------------------------------------------------
// DeepInfra Client
// ---------------------------------------------------------------------------

pub struct DeepInfraClient {
    pub base: BaseLlmClientData,
    pub api_url: String,
    pub api_key: String,
    pub http_client: reqwest::Client,
    pub formatter: Box<dyn PayloadFormatter>,
    pub model_type: DeepInfraModelType,
}

impl DeepInfraClient {
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
        let base = BaseLlmClientData::new(config_manager, model, spec, stdout, raw);

        // Detect model type from cache, falling back to name heuristics
        let cache_model_type = config_manager
            .model_type(provider_name, model)
            .unwrap_or(None);
        let model_type = DeepInfraModelType::from_model_id(model, cache_model_type.as_deref());
        let resolved_api_url = model_type.api_path(api_url, model);

        tracing::debug!(
            "DeepInfra client created: model={}, model_type={:?}, api_url={}",
            model,
            model_type,
            resolved_api_url
        );

        let http_client = create_http_client(config_manager)?;
        let formatter: Box<dyn PayloadFormatter> = Box::new(DeepInfraFormatter);

        Ok(Self {
            base,
            api_url: resolved_api_url,
            api_key: api_key.to_string(),
            http_client,
            formatter,
            model_type,
        })
    }

    /// Build the messages array for chat completion requests.
    #[must_use]
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

    /// Extract text content from the input data sources.
    fn extract_prompt(&self, data: &[DataSource]) -> String {
        let mut parts = Vec::new();
        for d in data {
            if d.content_type == "text/plain"
                && let Some(text) = d.content.as_str()
            {
                parts.push(text.to_string());
            }
        }
        // Also include the last user message from conversation history
        for msg in self.base.state.conversation.iter().rev() {
            if msg.role == Role::User {
                parts.push(msg.get_text(true));
                break;
            }
        }
        parts.join("\n")
    }

    /// Handle image generation requests via DeepInfra inference API.
    async fn send_image_generation(
        &mut self,
        data: Vec<DataSource>,
    ) -> anyhow::Result<LlmResponse> {
        let prompt = self.extract_prompt(&data);

        let body = json!({
            "input": {
                "prompt": prompt,
            }
        });

        tracing::debug!(
            "DeepInfra Image Generation Request (to {}): {}",
            self.api_url,
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );

        let res = self
            .http_client
            .post(&self.api_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("DeepInfra image generation request failed: {e}"))?;

        let resp_json: Value = res
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to parse DeepInfra response: {e}"))?;

        tracing::debug!(
            "DeepInfra Image Generation Response: {}",
            serde_json::to_string_pretty(&resp_json).unwrap_or_default()
        );

        if let Some(err) = resp_json.get("error") {
            return Err(anyhow::anyhow!("DeepInfra API Error: {err}"));
        }

        // DeepInfra image generation returns: {"output": [base64_string, ...]}
        // or sometimes {"output": base64_string}
        let output = resp_json.get("output").or_else(|| resp_json.get("images"));
        let images = match output {
            Some(Value::Array(arr)) => arr.clone(),
            Some(Value::String(s)) => vec![json!(s)],
            _ => Vec::new(),
        };

        if images.is_empty() {
            return Err(anyhow::anyhow!(
                "DeepInfra image generation returned no images. Response: {resp_json}"
            ));
        }

        // Build response with image data
        let mut message_parts = Vec::new();
        let mut text = String::new();

        for img in &images {
            let b64 = img.as_str().unwrap_or("");
            // Strip data URI prefix if returned by DeepInfra (e.g. "data:image/png;base64,...")
            let b64 = strip_data_uri_prefix(b64);
            if !b64.is_empty() {
                let mut inline = HashMap::new();
                inline.insert("mimeType".to_string(), json!("image/png"));
                inline.insert("data".to_string(), json!(b64));
                message_parts.push(MessagePart::Part(Box::new(
                    crate::llm::models::ContentPart {
                        inline_data: Some(inline),
                        ..Default::default()
                    },
                )));
                text.push_str(&format!(
                    "[Image generated: data:image/png;base64,{}, ...]\n",
                    &b64[..b64.len().min(50)]
                ));
            }
        }

        let model_msg = Message {
            role: Role::Assistant,
            parts: message_parts,
        };
        self.update_history(&data, model_msg);

        let usage = resp_json
            .get("usage")
            .and_then(|u| serde_json::from_value::<Usage>(u.clone()).ok());

        Ok(LlmResponse {
            content: Some(text.trim().to_string()),
            tool_name: None,
            usage,
        })
    }

    /// Handle video generation requests via DeepInfra inference API.
    async fn send_video_generation(
        &mut self,
        data: Vec<DataSource>,
    ) -> anyhow::Result<LlmResponse> {
        let prompt = self.extract_prompt(&data);

        let body = json!({
            "input": {
                "prompt": prompt,
            }
        });

        tracing::debug!(
            "DeepInfra Video Generation Request (to {}): {}",
            self.api_url,
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );

        let res = self
            .http_client
            .post(&self.api_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("DeepInfra video generation request failed: {e}"))?;

        let resp_json: Value = res
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to parse DeepInfra response: {e}"))?;

        if let Some(err) = resp_json.get("error") {
            return Err(anyhow::anyhow!("DeepInfra API Error: {err}"));
        }

        // DeepInfra video returns: {"output": base64_string} or {"output": [base64_string, ...]}
        let output = resp_json.get("output").or_else(|| resp_json.get("videos"));
        let videos = match output {
            Some(Value::Array(arr)) => arr.clone(),
            Some(Value::String(s)) => vec![json!(s)],
            _ => Vec::new(),
        };

        if videos.is_empty() {
            return Err(anyhow::anyhow!(
                "DeepInfra video generation returned no videos. Response: {resp_json}"
            ));
        }

        let mut message_parts = Vec::new();
        let mut text = String::new();

        for vid in &videos {
            if let Some(b64) = vid.as_str()
                && !b64.is_empty()
            {
                let mut inline = HashMap::new();
                inline.insert("mimeType".to_string(), json!("video/mp4"));
                inline.insert("data".to_string(), json!(b64));
                message_parts.push(MessagePart::Part(Box::new(
                    crate::llm::models::ContentPart {
                        inline_data: Some(inline),
                        ..Default::default()
                    },
                )));
                text.push_str(&format!(
                    "[Video generated: data:video/mp4;base64,{}, ...]\n",
                    &b64[..b64.len().min(50)]
                ));
            }
        }

        let model_msg = Message {
            role: Role::Assistant,
            parts: message_parts,
        };
        self.update_history(&data, model_msg);

        let usage = resp_json
            .get("usage")
            .and_then(|u| serde_json::from_value::<Usage>(u.clone()).ok());

        Ok(LlmResponse {
            content: Some(text.trim().to_string()),
            tool_name: None,
            usage,
        })
    }

    /// Handle audio generation/speech requests via DeepInfra inference API.
    async fn send_audio_generation(
        &mut self,
        data: Vec<DataSource>,
    ) -> anyhow::Result<LlmResponse> {
        let prompt = self.extract_prompt(&data);

        let body = json!({
            "input": {
                "text": prompt,
            }
        });

        tracing::debug!(
            "DeepInfra Audio Generation Request (to {}): {}",
            self.api_url,
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );

        let res = self
            .http_client
            .post(&self.api_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("DeepInfra audio generation request failed: {e}"))?;

        let resp_json: Value = res
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to parse DeepInfra response: {e}"))?;

        if let Some(err) = resp_json.get("error") {
            return Err(anyhow::anyhow!("DeepInfra API Error: {err}"));
        }

        // DeepInfra audio returns: {"output": base64_string} or {"audio": {...}}
        let audio_b64 = resp_json
            .get("output")
            .and_then(|v| v.as_str())
            .or_else(|| {
                resp_json
                    .get("audio")
                    .and_then(|v| v.get("data"))
                    .and_then(|v| v.as_str())
            })
            .unwrap_or("");

        if audio_b64.is_empty() {
            return Err(anyhow::anyhow!(
                "DeepInfra audio generation returned no audio data. Response: {resp_json}"
            ));
        }

        let mut inline = HashMap::new();
        inline.insert("mimeType".to_string(), json!("audio/wav"));
        inline.insert("data".to_string(), json!(audio_b64));
        let message_parts = vec![MessagePart::Part(Box::new(
            crate::llm::models::ContentPart {
                inline_data: Some(inline),
                ..Default::default()
            },
        ))];

        let text = format!(
            "[Audio generated: data:audio/wav;base64,{}, ...]",
            &audio_b64[..audio_b64.len().min(50)]
        );

        let model_msg = Message {
            role: Role::Assistant,
            parts: message_parts,
        };
        self.update_history(&data, model_msg);

        let usage = resp_json
            .get("usage")
            .and_then(|u| serde_json::from_value::<Usage>(u.clone()).ok());

        Ok(LlmResponse {
            content: Some(text),
            tool_name: None,
            usage,
        })
    }
}

// ---------------------------------------------------------------------------
// LlmClient trait implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl LlmClient for DeepInfraClient {
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
    ) -> anyhow::Result<LlmResponse> {
        match self.model_type {
            DeepInfraModelType::Chat => {
                // Standard OpenAI-compatible chat completion
                let messages = self.build_messages(&data);
                let mut body = json!({
                    "model": self.base.state.model,
                    "messages": messages,
                });

                if !tool_schemas.is_empty() {
                    body["tools"] = json!(
                        tool_schemas
                            .iter()
                            .map(|s| json!({"type": "function", "function": s}))
                            .collect::<Vec<_>>()
                    );
                }

                tracing::debug!(
                    "DeepInfra Chat Request (to {}): {}",
                    self.api_url,
                    serde_json::to_string_pretty(&body).unwrap_or_default()
                );

                let res = self
                    .http_client
                    .post(&self.api_url)
                    .header("Authorization", format!("Bearer {}", self.api_key))
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to send request to DeepInfra: {e}"))?;

                let resp_json: Value = res
                    .json()
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to parse DeepInfra response: {e}"))?;

                tracing::debug!(
                    "DeepInfra Chat Response: {}",
                    serde_json::to_string_pretty(&resp_json).unwrap_or_default()
                );

                if let Some(err) = resp_json.get("error") {
                    return Err(anyhow::anyhow!("DeepInfra API Error: {err}"));
                }

                let choice = &resp_json["choices"][0];
                let msg = &choice["message"];

                let usage = resp_json
                    .get("usage")
                    .and_then(|u| serde_json::from_value::<Usage>(u.clone()).ok());

                let parsed = response_parser::parse_assistant_message(msg);

                let model_msg = Message {
                    role: Role::Assistant,
                    parts: parsed.message_parts,
                };
                self.update_history(&data, model_msg);

                Ok(LlmResponse {
                    content: parsed.text,
                    tool_name: None,
                    usage,
                })
            }
            DeepInfraModelType::Image => self.send_image_generation(data).await,
            DeepInfraModelType::Video => self.send_video_generation(data).await,
            DeepInfraModelType::Audio => self.send_audio_generation(data).await,
        }
    }

    async fn send_as_verifier(
        &mut self,
        data: Vec<DataSource>,
        tool_schema: Value,
    ) -> anyhow::Result<Value> {
        // Verification only applies to chat models. For non-chat models, return an error.
        if self.model_type != DeepInfraModelType::Chat {
            return Err(anyhow::anyhow!(
                "DeepInfra verification is only supported for chat models, not {:?}",
                self.model_type
            ));
        }

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
            "DeepInfra Verifier Request (to {}): {}",
            self.api_url,
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );

        let res = self
            .http_client
            .post(&self.api_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send request to DeepInfra: {e}"))?;

        let resp_json: Value = res
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to parse DeepInfra response: {e}"))?;

        tracing::debug!(
            "DeepInfra Verifier Response: {}",
            serde_json::to_string_pretty(&resp_json).unwrap_or_default()
        );

        if let Some(err) = resp_json.get("error") {
            return Err(anyhow::anyhow!("DeepInfra API Error: {err}"));
        }

        let choice = match resp_json.get("choices").and_then(|c| c.get(0)) {
            Some(c) => c,
            None => {
                return Err(anyhow::anyhow!(
                    "Invalid response from DeepInfra: no choices found. Full response: {resp_json}"
                ));
            }
        };

        let msg = &choice["message"];

        if let Some(refusal) = msg.get("refusal").and_then(|v| v.as_str()) {
            return Err(anyhow::anyhow!("Model refused to verify: {refusal}"));
        }

        if let Some(tool_calls) = msg.get("tool_calls").and_then(|v| v.as_array())
            && !tool_calls.is_empty()
        {
            let tc = &tool_calls[0];
            let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
            return Ok(serde_json::from_str(args_str).unwrap_or(json!({})));
        }

        if let Some(content) = msg.get("content").and_then(|v| v.as_str()) {
            return Err(anyhow::anyhow!(
                "Verifier returned text instead of tool call: \"{content}\"."
            ));
        }

        Err(anyhow::anyhow!(
            "Verifier did not return a tool call. Raw response: {resp_json}"
        ))
    }
}
