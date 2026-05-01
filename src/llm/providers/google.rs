use crate::config::ConfigManager;
use crate::llm::base::{BaseLlmClientData, LlmClient, ProviderSpec};
use crate::llm::models::{ClientState, ContentPart, DataSource, Message, MessagePart, Role};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde_json::json;
use std::collections::HashMap;

static CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .build()
        .expect("Failed to create reqwest client")
});

pub struct GeminiClient {
    pub base: BaseLlmClientData,
}

impl GeminiClient {
    pub fn new(config_manager: &ConfigManager, model: &str, stdout: bool, raw: bool) -> Self {
        let spec = ProviderSpec {
            api_key_name: "api_key".to_string(),
            config_section: "google".to_string(),
            pdf_as_base64: true,
        };
        let base = BaseLlmClientData::new(config_manager, model, spec, stdout, raw);
        Self { base }
    }

    fn get_api_url(&self) -> String {
        let model = &self.base.state.model;
        let model_path = if model.contains('/') {
            model.to_string()
        } else {
            format!("models/{}", model)
        };
        format!(
            "https://generativelanguage.googleapis.com/v1beta/{}:generateContent",
            model_path
        )
    }

    /// Build the `contents` field for the Generate Content API from conversation history + new data.
    fn build_contents(&self, data: &[DataSource]) -> Vec<serde_json::Value> {
        let mut contents = Vec::new();

        for m in &self.base.state.conversation {
            let role_str = match m.role {
                Role::User | Role::Tool => "user",
                Role::Assistant | Role::Model => "model",
                Role::System => continue,
            };

            let parts = self.convert_message_parts_to_gemini(&m.parts);
            if !parts.is_empty() {
                contents.push(json!({
                    "role": role_str,
                    "parts": parts
                }));
            }
        }

        // Append new user data as a separate turn
        let new_parts = self.build_new_user_parts(data);
        if !new_parts.is_empty() {
            contents.push(json!({
                "role": "user",
                "parts": new_parts
            }));
        }

        contents
    }

    /// Convert MessageParts to Gemini API parts.
    fn convert_message_parts_to_gemini(&self, parts: &[MessagePart]) -> Vec<serde_json::Value> {
        let mut result = Vec::new();

        for part in parts {
            match part {
                MessagePart::Text(t) => {
                    result.push(json!({"text": t}));
                }
                MessagePart::Part(cp) => {
                    if let Some(t) = &cp.text
                        && !t.is_empty()
                    {
                        result.push(json!({"text": t}));
                    }

                    if let Some(fc) = &cp.function_call {
                        let name = fc.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let args = fc.get("arguments").cloned().unwrap_or(json!({}));
                        let mut function_call_part = json!({
                            "functionCall": {
                                "name": name,
                                "args": args
                            }
                        });
                        if let Some(sig) = &cp.thought_signature {
                            function_call_part["thought_signature"] = json!(sig);
                            function_call_part["thoughtSignature"] = json!(sig);
                        }
                        result.push(function_call_part);
                    }

                    if let Some(fr) = &cp.function_response {
                        let name = fr.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let response = fr.get("response").cloned().unwrap_or(json!({}));
                        let response = match response {
                            serde_json::Value::Object(_) => response,
                            other => json!({ "result": other }),
                        };
                        result.push(json!({
                            "functionResponse": {
                                "name": name,
                                "response": response
                            }
                        }));
                    }

                    if let Some(tc) = &cp.tool_call {
                        let mut tc_obj = json!({ "tool_call": tc });
                        if let Some(sig) = &cp.thought_signature {
                            tc_obj["thought_signature"] = json!(sig);
                            tc_obj["thoughtSignature"] = json!(sig);
                        }
                        result.push(tc_obj);
                    }

                    if let Some(tr) = &cp.tool_response {
                        result.push(json!({ "tool_response": tr }));
                    }

                    if let Some(t) = &cp.thought {
                        let mut thought_obj = json!({ "thought": t });
                        if let Some(sig) = &cp.thought_signature {
                            thought_obj["signature"] = json!(sig);
                        }
                        result.push(thought_obj);
                    }

                    if let Some(id) = &cp.inline_data {
                        let mime_type = id
                            .get("mimeType")
                            .or(id.get("mime_type"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let data = id.get("data").and_then(|v| v.as_str()).unwrap_or("");
                        result.push(json!({
                            "inline_data": {
                                "mime_type": mime_type,
                                "data": data
                            }
                        }));
                    }
                }
            }
        }

        result
    }

    /// Build the parts for new user data only.
    fn build_new_user_parts(&self, data: &[DataSource]) -> Vec<serde_json::Value> {
        let mut parts = Vec::new();
        for d in data {
            match d.content_type.as_str() {
                "text/plain" => {
                    parts.push(json!({
                        "text": d.content.as_str().unwrap_or("")
                    }));
                }
                ct if ct.starts_with("image/")
                    || ct.starts_with("audio/")
                    || ct.starts_with("video/")
                    || ct.starts_with("application/") =>
                {
                    parts.push(json!({
                        "inline_data": {
                            "mime_type": ct,
                            "data": d.content.as_str().unwrap_or("")
                        }
                    }));
                }
                _ => {
                    parts.push(json!({
                        "text": d.content.as_str().unwrap_or("")
                    }));
                }
            }
        }
        parts
    }

    /// Build tools for Generate Content API.
    fn build_tools(&self, tool_schemas: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
        let has_brave = tool_schemas.iter().any(|s| {
            s.get("name")
                .and_then(|v| v.as_str())
                .map(|s| s == "brave_search")
                .unwrap_or(false)
        });

        let mut tool_map = serde_json::Map::new();
        let mut function_declarations = Vec::new();

        for s in tool_schemas {
            function_declarations.push(json!({
                "name": s["name"],
                "description": s["description"],
                "parameters": s["parameters"]
            }));
        }

        if !function_declarations.is_empty() {
            tool_map.insert(
                "function_declarations".to_string(),
                json!(function_declarations),
            );
        }

        if !has_brave {
            // "google_search" is the current key for grounding in Gemini 2.0/3.0+
            tool_map.insert("google_search".to_string(), json!({}));
        }

        if tool_map.is_empty() {
            vec![]
        } else {
            vec![json!(tool_map)]
        }
    }

    /// Parse the Generate Content API response.
    fn parse_response(
        &self,
        res_json: &serde_json::Value,
    ) -> anyhow::Result<(String, String, Vec<MessagePart>, Option<String>)> {
        let mut full_text = String::new();
        let mut thought_text = String::new();
        let mut msg_parts = Vec::new();
        let mut current_thought_signature = None;

        let candidates = res_json["candidates"].as_array();
        if let Some(candidates) = candidates
            && let Some(candidate) = candidates.first()
        {
            if let Some(parts) = candidate["content"]["parts"].as_array() {
                for part in parts {
                    if let Some(text) = part["text"].as_str() {
                        full_text.push_str(text);
                        msg_parts.push(MessagePart::Part(Box::new(ContentPart {
                            text: Some(text.to_string()),
                            inline_data: None,
                            function_call: None,
                            function_response: None,
                            tool_call: None,
                            tool_response: None,
                            thought: None,
                            thought_signature: current_thought_signature.clone(),
                            is_diagnostic: false,
                        })));
                    } else if let Some(fc) = part.get("functionCall") {
                        let name = fc["name"].as_str().unwrap_or("").to_string();
                        let args = fc["args"].clone();
                        let thought_signature = part
                            .get("thoughtSignature")
                            .or_else(|| part.get("thought_signature"))
                            .or_else(|| fc.get("thought_signature"))
                            .or_else(|| fc.get("thoughtSignature"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                            .or_else(|| current_thought_signature.clone());

                        let mut function_call_map = HashMap::new();
                        function_call_map.insert("name".to_string(), json!(name));
                        function_call_map.insert("arguments".to_string(), args);

                        msg_parts.push(MessagePart::Part(Box::new(ContentPart {
                            text: None,
                            inline_data: None,
                            function_call: Some(function_call_map),
                            function_response: None,
                            tool_call: None,
                            tool_response: None,
                            thought: None,
                            thought_signature,
                            is_diagnostic: false,
                        })));
                    } else if let Some(tc) = part.get("toolCall") {
                        let tc_map = tc
                            .as_object()
                            .cloned()
                            .unwrap_or_default()
                            .into_iter()
                            .collect();
                        msg_parts.push(MessagePart::Part(Box::new(ContentPart {
                            text: None,
                            inline_data: None,
                            function_call: None,
                            function_response: None,
                            tool_call: Some(tc_map),
                            tool_response: None,
                            thought: None,
                            thought_signature: current_thought_signature.clone(),
                            is_diagnostic: false,
                        })));
                    } else if let Some(tr) = part.get("toolResponse") {
                        let tr_map = tr
                            .as_object()
                            .cloned()
                            .unwrap_or_default()
                            .into_iter()
                            .collect();
                        msg_parts.push(MessagePart::Part(Box::new(ContentPart {
                            text: None,
                            inline_data: None,
                            function_call: None,
                            function_response: None,
                            tool_call: None,
                            tool_response: Some(tr_map),
                            thought: None,
                            thought_signature: current_thought_signature.clone(),
                            is_diagnostic: false,
                        })));
                    } else if let Some(thought) = part.get("thought")
                        && let Some(t) = thought.as_str()
                    {
                        thought_text.push_str(t);
                        let signature = part
                            .get("signature")
                            .or_else(|| part.get("thought_signature"))
                            .or_else(|| part.get("thoughtSignature"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());

                        if signature.is_some() {
                            current_thought_signature = signature.clone();
                        }

                        msg_parts.push(MessagePart::Part(Box::new(ContentPart {
                            text: None,
                            inline_data: None,
                            function_call: None,
                            function_response: None,
                            tool_call: None,
                            tool_response: None,
                            thought: Some(t.to_string()),
                            thought_signature: signature,
                            is_diagnostic: false,
                        })));
                    }
                }
            }

            // Extract grounding metadata for citations
            if let Some(grounding) = candidate.get("groundingMetadata") {
                let mut citations = Vec::new();
                if let Some(chunks) = grounding
                    .get("groundingChunks")
                    .or(grounding.get("grounding_chunks"))
                    .and_then(|v| v.as_array())
                {
                    for chunk in chunks {
                        if let Some(web) = chunk.get("web") {
                            let title = web
                                .get("title")
                                .and_then(|v| v.as_str())
                                .unwrap_or("Source");
                            let uri = web.get("uri").and_then(|v| v.as_str()).unwrap_or("");
                            let clean_uri: String =
                                uri.chars().filter(|c| !c.is_whitespace()).collect();
                            if !clean_uri.is_empty() {
                                citations.push(format!("- [{}]({})", title, clean_uri));
                            }
                        }
                    }
                }
                if !citations.is_empty() {
                    let citations_text = format!("\n\n**Sources:**\n{}", citations.join("\n"));
                    full_text.push_str(&citations_text);
                    msg_parts.push(MessagePart::Text(citations_text));
                }
            }
        }

        // Check for finish reason if no text produced
        if full_text.is_empty()
            && thought_text.is_empty()
            && msg_parts.is_empty()
            && let Some(candidates) = candidates
            && let Some(candidate) = candidates.first()
            && let Some(reason) = candidate["finishReason"].as_str()
            && reason != "STOP"
            && reason != "stop"
        {
            full_text.push_str(&format!("[Finish reason: {}]", reason));
        }

        Ok((full_text, thought_text, msg_parts, None))
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

    fn should_send_pdf_as_base64(&self) -> bool {
        true
    }

    async fn send(
        &mut self,
        data: Vec<DataSource>,
        tool_schemas: Vec<serde_json::Value>,
    ) -> anyhow::Result<(Option<String>, Option<String>)> {
        let contents = self.build_contents(&data);
        let tools = self.build_tools(tool_schemas);

        let mut payload = json!({
            "contents": contents,
        });

        if let Some(sp_text) = self.base.state.get_effective_system_prompt() {
            payload["system_instruction"] = json!({
                "parts": [{ "text": sp_text }]
            });
        }

        if self.base.state.tools_enabled && !tools.is_empty() {
            payload["tools"] = json!(tools);
            // Required for combining built-in tools (google_search) and custom function calling
            payload["tool_config"] = json!({
                "include_server_side_tool_invocations": true
            });
        }

        log::debug!(
            "Gemini Request Payload: {}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );

        let mut retries = 0;
        let max_retries = 3;
        let mut backoff = std::time::Duration::from_secs(2);

        let res = loop {
            let url = self.get_api_url();
            let key = self.base.api_key.as_deref().unwrap_or("").to_string();

            let res_result = CLIENT
                .post(&url)
                .header("x-goog-api-key", key)
                .json(&payload)
                .send()
                .await;

            match res_result {
                Ok(r) => break r,
                Err(e) => {
                    let status_code = e.status().map(|s| s.as_u16()).unwrap_or(0);
                    if status_code == 429 && retries < max_retries {
                        log::warn!(
                            "Gemini API rate limit (429) hit. Retrying in {:?}...",
                            backoff
                        );
                        tokio::time::sleep(backoff).await;
                        retries += 1;
                        backoff *= 2;
                        continue;
                    }
                    return Err(anyhow::anyhow!("Gemini API request failed: {}", e));
                }
            }
        };

        let status = res.status();
        let res_json: serde_json::Value = res.json().await.unwrap_or_default();

        if !status.is_success() {
            let err_msg = res_json["error"]["message"]
                .as_str()
                .unwrap_or("Unknown error");
            return Err(anyhow::anyhow!(
                "Gemini API error ({}): {}",
                status,
                err_msg
            ));
        }

        let (full_text, thought_text, msg_parts, interaction_id) =
            self.parse_response(&res_json)?;

        let model_msg = Message {
            role: Role::Model,
            parts: msg_parts,
        };
        self.update_history(&data, model_msg);
        self.base.state.previous_interaction_id = interaction_id;

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

    async fn send_as_verifier(
        &mut self,
        data: Vec<DataSource>,
        tool_schema: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let contents = self.build_contents(&data);
        let tool_name = tool_schema["name"].as_str().unwrap_or("verify").to_string();

        let mut payload = json!({
            "contents": contents,
            "tools": [{
                "function_declarations": [tool_schema]
            }],
            "tool_config": {
                "function_calling_config": {
                    "mode": "ANY",
                    "allowed_function_names": [tool_name]
                }
            }
        });

        if let Some(sp_text) = self.base.state.get_effective_system_prompt() {
            payload["system_instruction"] = json!({
                "parts": [{ "text": sp_text }]
            });
        }

        let url = self.get_api_url();
        let key = self.base.api_key.as_deref().unwrap_or("").to_string();

        let res = CLIENT
            .post(&url)
            .header("x-goog-api-key", key)
            .json(&payload)
            .send()
            .await?;

        if !res.status().is_success() {
            let res_json: serde_json::Value = res.json().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Gemini verifier error: {}", res_json));
        }

        let res_json: serde_json::Value = res.json().await?;

        let args = res_json["candidates"][0]["content"]["parts"]
            .as_array()
            .and_then(|parts| {
                parts
                    .iter()
                    .find_map(|p| p.get("functionCall").map(|fc| fc["args"].clone()))
            })
            .ok_or_else(|| anyhow::anyhow!("No functionCall in Gemini verifier response"))?;

        Ok(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::models::{ClientState, Message, MessagePart, Role};

    fn test_client(conversation: Vec<Message>) -> GeminiClient {
        GeminiClient {
            base: BaseLlmClientData {
                state: ClientState {
                    model: "gemini-1.5-flash".to_string(),
                    provider: "google".to_string(),
                    conversation,
                    tools_enabled: true,
                    system_prompt_enabled: true,
                    system_prompt: None,
                    stdout: false,
                    render_markdown: true,
                    live_debug: false,
                    previous_interaction_id: None,
                },
                config_section: "google".to_string(),
                api_key: None,
            },
        }
    }

    #[test]
    fn build_contents_structure() {
        let client = test_client(vec![Message {
            role: Role::User,
            parts: vec![MessagePart::Text("Hello".to_string())],
        }]);

        let contents = client.build_contents(&[]);
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0]["role"], "user");
        assert_eq!(contents[0]["parts"][0]["text"], "Hello");
    }

    #[test]
    fn build_tools_structure() {
        let client = test_client(vec![]);
        let tool_schemas = vec![json!({
            "name": "test_tool",
            "description": "A test tool",
            "parameters": {
                "type": "object",
                "properties": {
                    "arg1": {"type": "string"}
                }
            }
        })];

        let tools = client.build_tools(tool_schemas);
        assert_eq!(tools.len(), 1); // Combined into one tool object
        assert!(tools[0].get("function_declarations").is_some());
        assert_eq!(tools[0]["function_declarations"][0]["name"], "test_tool");
        assert!(tools[0].get("google_search").is_some());
    }
}
