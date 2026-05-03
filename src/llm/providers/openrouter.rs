use crate::config::ConfigManager;
use crate::llm::base::LlmClient;
use crate::llm::models::{ClientState, ContentPart, DataSource, Message, MessagePart, Role};
use crate::llm::providers::openai_compatible::OpenAiCompatibleClient;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::HashMap;

pub struct OpenRouterClient {
    inner: OpenAiCompatibleClient,
}

impl OpenRouterClient {
    pub fn new(
        config_manager: &ConfigManager,
        provider_name: &str,
        api_url: &str,
        api_key: &str,
        model: &str,
        stdout: bool,
        raw: bool,
    ) -> anyhow::Result<Self> {
        let inner = OpenAiCompatibleClient::new(
            config_manager,
            provider_name,
            api_url,
            api_key,
            model,
            stdout,
            raw,
        )?;
        Ok(Self { inner })
    }
}

#[async_trait]
impl LlmClient for OpenRouterClient {
    fn get_state(&self) -> &ClientState {
        self.inner.get_state()
    }

    fn get_state_mut(&mut self) -> &mut ClientState {
        self.inner.get_state_mut()
    }

    fn get_config_section(&self) -> &str {
        self.inner.get_config_section()
    }

    fn should_send_pdf_as_base64(&self) -> bool {
        self.inner.should_send_pdf_as_base64()
    }

    async fn send(
        &mut self,
        data: Vec<DataSource>,
        tool_schemas: Vec<Value>,
    ) -> anyhow::Result<(Option<String>, Option<String>)> {
        let messages = self.inner.build_messages(&data);

        let mut request_url = self.inner.api_url.clone();

        // If the model is exclusively a video or audio processing model, we override the endpoint
        if self.inner.video_generation_enabled {
            request_url = request_url.replace("/chat/completions", "/videos");
        } else if self.inner.audio_generation_enabled {
            request_url = request_url.replace("/chat/completions", "/audio/speech");
        }

        let body = if self.inner.video_generation_enabled {
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
                "model": self.inner.base.state.model,
                "prompt": prompt,
            });

            // Extract frame images or input references from user messages if they exist
            let mut images = Vec::new();
            for m in &messages {
                if let Some(content) = m.get("content").and_then(|v| v.as_array()) {
                    for p in content {
                        if p.get("type").and_then(|v| v.as_str()) == Some("image_url")
                            && let Some(image_url) = p.get("image_url")
                        {
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
        } else if self.inner.audio_generation_enabled {
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
                "model": self.inner.base.state.model,
                "input": input,
                // "voice": ... typically required but depends on provider. "alloy" works for OpenAI TTS.
                // You can add logic to extract voice from settings or prompt if needed.
                "voice": "alloy",
            })
        } else {
            // Standard /chat/completions payload
            let mut req = json!({
                "model": self.inner.base.state.model,
                "messages": messages,
            });

            let mut tools = if self.inner.supports_tools && self.inner.base.state.tools_enabled {
                self.inner.build_tool_schemas(tool_schemas)
            } else {
                Vec::new()
            };
            if self.inner.supports_tools && self.inner.image_generation_enabled {
                tools.push(json!({ "type": "image_generation" }));
            }
            if !tools.is_empty() {
                req["tools"] = json!(tools);
                req["tool_choice"] = json!("auto");
            }

            if self.inner.image_generation_enabled {
                req["modalities"] = json!(["image"]);
            }
            req
        };

        let res = self
            .inner
            .http_client
            .post(&request_url)
            .header("Authorization", format!("Bearer {}", self.inner.api_key))
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

        if !is_json && self.inner.audio_generation_enabled {
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
            self.inner.update_history(&data, model_msg);
            return Ok((Some(text), None));
        }

        let resp: Value = res.json().await?;

        if let Some(error) = resp.get("error") {
            return Err(anyhow::anyhow!("API Error: {}", error));
        }

        // OpenRouter /api/v1/videos response returns job ID and polling URL
        if self.inner.video_generation_enabled
            && let Some(job_id) = resp.get("id").and_then(|v| v.as_str())
        {
            let mut text = format!("Video generation submitted. Job ID: {}\n", job_id);
            if let Some(polling_url) = resp.get("polling_url").and_then(|v| v.as_str()) {
                let mut delay = std::time::Duration::from_secs(5);
                let max_delay = std::time::Duration::from_secs(30);

                loop {
                    use std::io::Write;
                    if !self.inner.base.state.stdout {
                        eprint!("\rPolling video generation status...");
                        let _ = std::io::stderr().flush();
                    }
                    tokio::time::sleep(delay).await;

                    let poll_res = self
                        .inner
                        .http_client
                        .get(polling_url)
                        .header("Authorization", format!("Bearer {}", self.inner.api_key))
                        .send()
                        .await;

                    match poll_res {
                        Ok(res) => {
                            let poll_body: Value = res.json().await.unwrap_or(json!({}));
                            if let Some(status) = poll_body.get("status").and_then(|v| v.as_str()) {
                                if status == "completed" {
                                    if let Some(video_url) =
                                        poll_body.get("video_url").and_then(|v| v.as_str())
                                    {
                                        if !self.inner.base.state.stdout {
                                            eprintln!(
                                                "\rPolling video generation status... Completed!                           "
                                            );
                                        }
                                        text = format!(
                                            "Video generation completed.\nVideo URL: {}",
                                            video_url
                                        );
                                        break;
                                    }
                                } else if status == "failed" {
                                    if !self.inner.base.state.stdout {
                                        eprintln!(
                                            "\rPolling video generation status... Failed!                              "
                                        );
                                    }
                                    let error = poll_body
                                        .get("error")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("Unknown error");
                                    text = format!("Video generation failed: {}", error);
                                    break;
                                } else {
                                    // Still processing
                                    if delay < max_delay {
                                        delay += std::time::Duration::from_secs(5);
                                    }
                                }
                            }
                        }
                        Err(_) => {
                            // Ignore temporary network errors while polling
                        }
                    }
                }
            }

            let model_msg = Message {
                role: Role::Assistant,
                parts: vec![MessagePart::Text(text.clone())],
            };
            self.inner.update_history(&data, model_msg);
            return Ok((Some(text), None));
        }

        // Delegate parsing logic to openai_compatible
        self.inner.parse_response(resp, status, data)
    }

    async fn send_as_verifier(
        &mut self,
        data: Vec<DataSource>,
        tool_schema: Value,
    ) -> anyhow::Result<Value> {
        self.inner.send_as_verifier(data, tool_schema).await
    }
}
