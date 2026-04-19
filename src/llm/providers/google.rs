use crate::llm::base::{BaseLlmClientData, LlmClient, ProviderSpec};
use crate::llm::models::{ClientState, ContentPart, DataSource, Message, MessagePart, Role};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use reqwest::Client;
use serde_json::json;
use std::collections::HashMap;

static HTTP_CLIENT: Lazy<Client> = Lazy::new(Client::new);

pub struct GeminiClient {
    pub base: BaseLlmClientData,
}

impl GeminiClient {
    pub fn new(model: &str, stdout: bool, raw: bool) -> Self {
        let spec = ProviderSpec {
            api_key_name: "api_key".to_string(),
            config_section: "google".to_string(),
            pdf_as_base64: true,
        };
        let base = BaseLlmClientData::new(model, spec, stdout, raw);
        Self { base }
    }

    fn get_api_url(&self) -> String {
        let model = &self.base.state.model;
        let key = self.base.api_key.as_deref().unwrap_or("");
        format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            model, key
        )
    }

    fn build_contents(&self, data: &[DataSource]) -> Vec<serde_json::Value> {
        let mut contents = Vec::new();

        for m in &self.base.state.conversation {
            let role = match m.role {
                Role::User => "user",
                Role::Assistant | Role::Model => "model",
                Role::Tool => "user", // tool results are sent as user role in Gemini
                Role::System => continue,
            };

            let mut parts = Vec::new();
            let mut prev_thought_sig: Option<String> = None;

            for part in &m.parts {
                match part {
                    MessagePart::Text(t) => {
                        parts.push(json!({"text": t}));
                        prev_thought_sig = None;
                    }
                    MessagePart::Part(cp) => {
                        let thought_sig = cp.thought_signature.clone();

                        if let Some(thought) = &cp.thought {
                            let mut part_json = json!({
                                "thought": true,
                                "text": thought
                            });
                            if let Some(sig) = &thought_sig {
                                part_json["thoughtSignature"] = json!(sig);
                            }
                            parts.push(part_json);

                            if let Some(t) = &cp.text {
                                if !t.is_empty() {
                                    let mut text_json = json!({"text": t});
                                    let effective_sig =
                                        thought_sig.clone().or(prev_thought_sig.clone());
                                    if let Some(sig) = effective_sig {
                                        text_json["thoughtSignature"] = json!(sig);
                                    }
                                    parts.push(text_json);
                                }
                            }
                            prev_thought_sig = thought_sig.clone();
                        } else if let Some(t) = &cp.text {
                            if !t.is_empty() {
                                let mut text_json = json!({"text": t});
                                let effective_sig =
                                    thought_sig.clone().or(prev_thought_sig.clone());
                                if let Some(sig) = effective_sig {
                                    text_json["thoughtSignature"] = json!(sig);
                                }
                                parts.push(text_json);
                                prev_thought_sig = None;
                            }
                        }

                        if let Some(fc) = &cp.function_call {
                            let name = fc.get("name").and_then(|v| v.as_str()).unwrap_or("");
                            let args = fc.get("arguments").cloned().unwrap_or(json!({}));
                            let mut fc_part = json!({
                                "functionCall": {
                                    "name": name,
                                    "args": args
                                }
                            });
                            let effective_sig = thought_sig.clone().or(prev_thought_sig.clone());
                            if let Some(sig) = effective_sig {
                                fc_part["thoughtSignature"] = json!(sig);
                            }
                            parts.push(fc_part);
                            prev_thought_sig = None;
                        }

                        if let Some(fr) = &cp.function_response {
                            let name = fr.get("name").and_then(|v| v.as_str()).unwrap_or("");
                            let response = fr.get("response").cloned().unwrap_or(json!({}));
                            let wrapped_response = if response.is_object() {
                                response
                            } else {
                                json!({ "result": response })
                            };
                            parts.push(json!({
                                "functionResponse": {
                                    "name": name,
                                    "response": wrapped_response
                                }
                            }));
                            prev_thought_sig = None;
                        }

                        if let Some(id) = &cp.inline_data {
                            let mut id_part = json!({"inlineData": id});
                            if let Some(sig) = &thought_sig {
                                id_part["thoughtSignature"] = json!(sig);
                            }
                            parts.push(id_part);
                            prev_thought_sig = None;
                        }
                    }
                }
            }

            if !parts.is_empty() {
                contents.push(json!({
                    "role": role,
                    "parts": parts
                }));
            }
        }

        // Add new user data
        let mut new_parts = Vec::new();
        for d in data {
            match d.content_type.as_str() {
                "text/plain" => {
                    new_parts.push(json!({"text": d.content.as_str().unwrap_or("")}));
                }
                ct if ct.starts_with("image/") || ct.starts_with("application/") => {
                    new_parts.push(json!({
                        "inlineData": {
                            "mimeType": ct,
                            "data": d.content.as_str().unwrap_or("")
                        }
                    }));
                }
                _ => {
                    new_parts.push(json!({"text": d.content.as_str().unwrap_or("")}));
                }
            }
        }

        if !new_parts.is_empty() {
            contents.push(json!({
                "role": "user",
                "parts": new_parts
            }));
        }

        contents
    }

    fn build_tool_declarations(&self) -> Vec<serde_json::Value> {
        let registry = crate::tools::registry::REGISTRY.lock().unwrap();
        registry.get_tool_schemas_gemini()
    }
}

#[async_trait]
impl LlmClient for GeminiClient {
    fn get_state(&self) -> &ClientState {
        &self.base.state
    }
    fn get_state_mut(&mut self) -> &mut ClientState {
        &mut self.base.state
    }
    fn get_config_section(&self) -> &str {
        &self.base.config_section
    }

    fn get_display_name(&self) -> String {
        format!("GEMINI ({})", self.base.state.model)
    }

    async fn send(
        &mut self,
        data: Vec<DataSource>,
    ) -> anyhow::Result<(Option<String>, Option<String>)> {
        let contents = self.build_contents(&data);
        let tool_declarations = self.build_tool_declarations();

        // Get system prompt from config
        let system_prompt = crate::config::CONFIG_MANAGER
            .get_config()
            .providers
            .get("google")
            .and_then(|p| p.system_prompt.clone());

        let mut payload = json!({
            "contents": contents,
        });

        if let Some(sp) = system_prompt {
            if !sp.is_empty() {
                payload["system_instruction"] = json!({
                    "parts": [{"text": sp}]
                });
            }
        }

        if !tool_declarations.is_empty() {
            payload["tools"] = json!([{
                "function_declarations": tool_declarations
            }]);
        }

        log::debug!(
            "Gemini Request Payload: {}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );

        let url = self.get_api_url();

        let res = HTTP_CLIENT.post(&url).json(&payload).send().await?;

        let status = res.status();
        let res_json: serde_json::Value = res.json().await?;
        log::debug!(
            "Gemini Response ({}): {}",
            status,
            serde_json::to_string_pretty(&res_json).unwrap_or_default()
        );

        if !status.is_success() {
            let err_msg = res_json["error"]["message"]
                .as_str()
                .unwrap_or("Unknown error")
                .to_string();
            return Err(anyhow::anyhow!(
                "Gemini API error ({}): {}",
                status,
                err_msg
            ));
        }

        let mut full_text = String::new();
        let mut thought_text = String::new();
        let mut msg_parts = Vec::new();

        if let Some(candidates) = res_json["candidates"].as_array() {
            if let Some(candidate) = candidates.first() {
                if let Some(parts) = candidate["content"]["parts"].as_array() {
                    for part in parts {
                        let thought_sig = part
                            .get("thoughtSignature")
                            .or(part.get("thought_signature"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());

                        if part
                            .get("thought")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                        {
                            let text = part
                                .get("text")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            thought_text.push_str(&text);
                            msg_parts.push(MessagePart::Part(ContentPart {
                                text: None,
                                inline_data: None,
                                function_call: None,
                                function_response: None,
                                thought: Some(text),
                                thought_signature: thought_sig,
                                is_diagnostic: false,
                            }));
                        } else if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                            full_text.push_str(text);
                            msg_parts.push(MessagePart::Part(ContentPart {
                                text: Some(text.to_string()),
                                inline_data: None,
                                function_call: None,
                                function_response: None,
                                thought: None,
                                thought_signature: thought_sig,
                                is_diagnostic: false,
                            }));
                        } else if let Some(fc) = part.get("functionCall") {
                            let name = fc["name"].as_str().unwrap_or("").to_string();
                            let args = fc["args"].clone();
                            let mut function_call_map = HashMap::new();
                            function_call_map.insert("name".to_string(), json!(name));
                            function_call_map.insert("arguments".to_string(), args);
                            function_call_map
                                .insert("id".to_string(), json!(format!("gemini-fc-{}", name)));

                            msg_parts.push(MessagePart::Part(ContentPart {
                                text: None,
                                inline_data: None,
                                function_call: Some(function_call_map),
                                function_response: None,
                                thought: None,
                                thought_signature: thought_sig,
                                is_diagnostic: false,
                            }));
                        }
                    }
                }
            }
        }

        let model_msg = Message {
            role: Role::Model,
            parts: msg_parts,
        };

        self.update_history(&data, model_msg);

        Ok((
            if full_text.is_empty() {
                None
            } else {
                Some(full_text)
            },
            if thought_text.is_empty() {
                None
            } else {
                Some(thought_text)
            },
        ))
    }
}
