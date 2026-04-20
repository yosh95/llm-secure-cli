use crate::llm::base::{BaseLlmClientData, LlmClient, ProviderSpec};
use crate::llm::models::{ClientState, ContentPart, DataSource, Message, MessagePart, Role};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use reqwest::Client;
use serde_json::json;
use std::collections::HashMap;

static HTTP_CLIENT: Lazy<Client> = Lazy::new(Client::new);
const IMAGE_API_URL: &str = "https://api.openai.com/v1/images/generations";

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

    fn is_image_model(&self) -> bool {
        let model = self.base.state.model.to_lowercase();
        model.contains("dall-e") || model.contains("image")
    }

    fn build_prompt_from_history(&self, data: &[DataSource]) -> String {
        let mut prompt_parts = Vec::new();
        for msg in &self.base.state.conversation {
            for part in &msg.parts {
                match part {
                    MessagePart::Text(t) => prompt_parts.push(t.clone()),
                    MessagePart::Part(cp) => {
                        if let Some(t) = &cp.text {
                            prompt_parts.push(t.clone());
                        }
                    }
                }
            }
        }
        for d in data {
            if d.content_type == "text/plain" {
                if let Some(t) = d.content.as_str() {
                    prompt_parts.push(t.to_string());
                }
            }
        }
        prompt_parts.join("\n")
    }

    async fn send_image_generation(
        &mut self,
        data: Vec<DataSource>,
    ) -> anyhow::Result<(Option<String>, Option<String>)> {
        let full_prompt = self.build_prompt_from_history(&data);
        let payload = json!({
            "model": self.base.state.model,
            "prompt": full_prompt,
            "n": 1,
            "size": "1024x1024",
        });

        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(key) = &self.base.api_key {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", key).parse()?,
            );
        }

        let res = HTTP_CLIENT
            .post(IMAGE_API_URL)
            .headers(headers)
            .json(&payload)
            .send()
            .await?;

        if !res.status().is_success() {
            let err_text = res.text().await?;
            return Err(anyhow::anyhow!("OpenAI Image API error: {}", err_text));
        }

        let res_json: serde_json::Value = res.json().await?;

        let data_item = res_json["data"]
            .get(0)
            .ok_or_else(|| anyhow::anyhow!("No image data in response"))?;
        let revised_prompt = data_item.get("revised_prompt").and_then(|v| v.as_str());
        let mut img_data = None;
        let mut mime_type = "image/png".to_string();

        if let Some(b64) = data_item.get("b64_json").and_then(|v| v.as_str()) {
            img_data = Some(b64.to_string());
        } else if let Some(url) = data_item.get("url").and_then(|v| v.as_str()) {
            let (fetched_data, fetched_mime) =
                crate::utils::media::fetch_url_content(url, true).await?;
            img_data = Some(fetched_data);
            mime_type = fetched_mime;
        }

        let Some(img_data_str) = img_data else {
            return Ok((Some("Failed to retrieve image data.".to_string()), None));
        };

        let save_path = &crate::config::CONFIG_MANAGER
            .get_config()
            .general
            .image_save_path;
        let saved_path = crate::utils::media::save_image(&img_data_str, &mime_type, save_path)?;

        let mut display_text = format!("Image saved to {}", saved_path);
        if let Some(rp) = revised_prompt {
            display_text.push_str(&format!("\n\n**Revised Prompt:** {}", rp));
        }

        let mut inline_data = HashMap::new();
        inline_data.insert("mimeType".to_string(), json!(mime_type));
        inline_data.insert("data".to_string(), json!(img_data_str));

        let model_msg = Message {
            role: Role::Assistant,
            parts: vec![
                MessagePart::Text(display_text.clone()),
                MessagePart::Part(ContentPart {
                    text: None,
                    inline_data: Some(inline_data),
                    function_call: None,
                    function_response: None,
                    thought: None,
                    thought_signature: None,
                    is_diagnostic: false,
                }),
            ],
        };

        self.update_history(&data, model_msg);

        Ok((Some(display_text), None))
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
            let mut content_parts = Vec::new();
            let mut tool_calls = Vec::new();

            for part in &m.parts {
                match part {
                    MessagePart::Text(t) => {
                        content_parts.push(json!({"type": "text", "text": t}));
                    }
                    MessagePart::Part(cp) => {
                        if let Some(t) = &cp.text {
                            content_parts.push(json!({"type": "text", "text": t}));
                        }
                        if let Some(id) = &cp.inline_data {
                            if let (Some(mime), Some(data)) = (id.get("mimeType"), id.get("data")) {
                                content_parts.push(json!({
                                    "type": "image_url",
                                    "image_url": {
                                        "url": format!("data:{};base64,{}", mime.as_str().unwrap_or(""), data.as_str().unwrap_or(""))
                                    }
                                }));
                            }
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
                            content_parts.push(json!({"type": "text", "text": response_str}));
                        }
                    }
                }
            }

            // If it's a simple text message, use the string format for better compatibility
            // Otherwise use the array format for multimodal support
            if !content_parts.is_empty() {
                if content_parts.len() == 1 && content_parts[0]["type"] == "text" {
                    msg["content"] = content_parts[0]["text"].clone();
                } else {
                    msg["content"] = json!(content_parts);
                }
            } else if role == "tool" {
                msg["content"] = json!("");
            } else if role == "assistant" && !tool_calls.is_empty() {
                msg["content"] = json!(null);
            }

            if !tool_calls.is_empty() {
                msg["tool_calls"] = json!(tool_calls);
            }

            messages.push(msg);
        }

        // Handle new data from DataSource
        let mut new_parts = Vec::new();
        for d in data {
            if d.content_type == "text/plain" {
                new_parts.push(json!({"type": "text", "text": d.content.as_str().unwrap_or("")}));
            } else if d.content_type.starts_with("image/") {
                new_parts.push(json!({
                    "type": "image_url",
                    "image_url": {
                        "url": format!("data:{};base64,{}", d.content_type, d.content.as_str().unwrap_or(""))
                    }
                }));
            }
        }

        if !new_parts.is_empty() {
            if new_parts.len() == 1 && new_parts[0]["type"] == "text" {
                messages.push(json!({
                    "role": "user",
                    "content": new_parts[0]["text"].clone()
                }));
            } else {
                messages.push(json!({
                    "role": "user",
                    "content": new_parts
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
        if self.is_image_model() {
            return self.send_image_generation(data).await;
        }

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

        if self.base.state.tools_enabled && !tool_schemas.is_empty() {
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

        let status = res.status();
        let res_json: serde_json::Value = res.json().await?;
        log::debug!(
            "OpenAI Response ({}): {}",
            status,
            serde_json::to_string_pretty(&res_json).unwrap_or_default()
        );

        if !status.is_success() {
            if let Some(err) = res_json.get("error") {
                return Err(anyhow::anyhow!("OpenAI API error ({}): {}", status, err));
            } else {
                return Err(anyhow::anyhow!(
                    "OpenAI API error ({}): {}",
                    status,
                    res_json
                ));
            }
        }

        let choice = &res_json["choices"][0];
        let message = &choice["message"];

        // Extract text
        let mut full_text = String::new();
        if let Some(t) = message["content"].as_str() {
            full_text.push_str(t);
        }

        let mut model_parts = Vec::new();
        if !full_text.is_empty() {
            model_parts.push(MessagePart::Text(full_text.clone()));
        }

        // Handle OpenAI-style content parts (some models return array of parts including images)
        if let Some(parts) = message["content"].as_array() {
            for part in parts {
                if let Some(t) = part["text"].as_str() {
                    if full_text.is_empty() {
                        full_text.push_str(t);
                    }
                    model_parts.push(MessagePart::Text(t.to_string()));
                }
                if let Some(img) = part["image_url"].as_object() {
                    let url = img.get("url").and_then(|v| v.as_str()).unwrap_or("");
                    if url.starts_with("data:image/") {
                        if let Some(comma_pos) = url.find(',') {
                            let mime = url[5..comma_pos].split(';').next().unwrap_or("");
                            let data = &url[comma_pos + 1..];
                            full_text.push_str(&format!("[Image: {}]", mime));

                            let mut inline_data = HashMap::new();
                            inline_data.insert("mimeType".to_string(), json!(mime));
                            inline_data.insert("data".to_string(), json!(data));

                            model_parts.push(MessagePart::Part(ContentPart {
                                text: None,
                                inline_data: Some(inline_data),
                                function_call: None,
                                function_response: None,
                                thought: None,
                                thought_signature: None,
                                is_diagnostic: false,
                            }));
                        }
                    } else if !url.is_empty() {
                        full_text.push_str(&format!("[Image URL: {}]", url));
                    }
                }
            }
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

        Ok((
            if full_text.is_empty() {
                None
            } else {
                Some(full_text)
            },
            None,
        ))
    }
}
