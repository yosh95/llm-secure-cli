use crate::llm::base::{self, BaseLlmClientData, LlmClient, ProviderSpec};
use crate::llm::models::{ClientState, ContentPart, DataSource, Message, MessagePart, Role};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde_json::json;
use std::collections::HashMap;

static CLIENT: Lazy<reqwest::Client> = Lazy::new(base::create_http_client);

pub struct GeminiClient {
    pub base: BaseLlmClientData,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::models::{ClientState, ContentPart, Message, MessagePart, Role};
    use std::collections::HashMap;

    fn test_client(conversation: Vec<Message>) -> GeminiClient {
        GeminiClient {
            base: BaseLlmClientData {
                state: ClientState {
                    model: "gemini-3-flash-preview".to_string(),
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
    fn build_input_omits_standalone_thought_parts_from_history() {
        let mut fc = HashMap::new();
        fc.insert("name".to_string(), json!("execute_command"));
        fc.insert("id".to_string(), json!("call-1"));
        fc.insert(
            "arguments".to_string(),
            json!({"command": "git", "args": ["status"]}),
        );

        let client = test_client(vec![
            Message {
                role: Role::User,
                parts: vec![MessagePart::Text("git commit".to_string())],
            },
            Message {
                role: Role::Model,
                parts: vec![
                    MessagePart::Part(ContentPart {
                        text: None,
                        inline_data: None,
                        function_call: None,
                        function_response: None,
                        thought: Some(String::new()),
                        thought_signature: None,
                        is_diagnostic: false,
                    }),
                    MessagePart::Part(ContentPart {
                        text: None,
                        inline_data: None,
                        function_call: Some(fc),
                        function_response: None,
                        thought: None,
                        thought_signature: None,
                        is_diagnostic: false,
                    }),
                ],
            },
        ]);

        let input = client.build_input(&[DataSource {
            content: json!("git commit"),
            content_type: "text/plain".to_string(),
            is_file_or_url: false,
            metadata: HashMap::new(),
        }]);

        let serialized = serde_json::to_string(&input).unwrap();
        assert!(!serialized.contains("\"type\":\"thought\""));
        assert!(serialized.contains("\"type\":\"function_call\""));
        assert!(serialized.contains("\"type\":\"text\""));
    }

    #[test]
    fn build_input_carries_thought_signature_to_next_concrete_part() {
        let mut fc = HashMap::new();
        fc.insert("name".to_string(), json!("execute_command"));
        fc.insert("id".to_string(), json!("call-1"));
        fc.insert("arguments".to_string(), json!({"command": "git"}));

        let client = test_client(vec![Message {
            role: Role::Model,
            parts: vec![
                MessagePart::Part(ContentPart {
                    text: None,
                    inline_data: None,
                    function_call: None,
                    function_response: None,
                    thought: Some(String::new()),
                    thought_signature: Some("sig-1".to_string()),
                    is_diagnostic: false,
                }),
                MessagePart::Part(ContentPart {
                    text: None,
                    inline_data: None,
                    function_call: Some(fc),
                    function_response: None,
                    thought: None,
                    thought_signature: None,
                    is_diagnostic: false,
                }),
            ],
        }]);

        let input = client.build_input(&[]);
        let model_content = input[0]["content"].as_array().unwrap();

        assert_eq!(model_content.len(), 1);
        assert_eq!(model_content[0]["type"], "function_call");
        assert_eq!(model_content[0]["thought_signature"], "sig-1");
    }
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
        "https://generativelanguage.googleapis.com/v1beta/interactions".to_string()
    }

    /// Build the `input` field for the Interactions API from conversation history + new data.
    /// Returns a Vec of content objects (or turns) in Interactions API format.
    fn build_input(&self, data: &[DataSource]) -> serde_json::Value {
        // If we have a previous_interaction_id, we only send the new data — the server
        // already knows the history and can serve from cache.
        if self.base.state.previous_interaction_id.is_some() {
            return self.build_new_user_content(data);
        }

        // Otherwise, send the full conversation history as input turns.
        let mut turns = Vec::new();

        // Group conversation messages into user/model turns
        let mut current_role: Option<String> = None;
        let mut current_parts: Vec<serde_json::Value> = Vec::new();

        for m in &self.base.state.conversation {
            let role_str = match m.role {
                Role::User | Role::Tool => "user",
                Role::Assistant | Role::Model => "model",
                Role::System => continue,
            };

            let parts = self.convert_message_parts_to_interactions(&m.parts);

            if current_role.as_deref() == Some(role_str) {
                // Same role — merge parts into the same turn
                current_parts.extend(parts);
            } else {
                // Flush previous turn
                if !current_parts.is_empty()
                    && let Some(role) = current_role.take()
                {
                    turns.push(json!({
                        "role": role,
                        "content": std::mem::take(&mut current_parts)
                    }));
                }
                current_role = Some(role_str.to_string());
                current_parts = parts;
            }
        }

        // Flush last turn
        if !current_parts.is_empty()
            && let Some(role) = current_role
        {
            turns.push(json!({
                "role": role,
                "content": current_parts
            }));
        }

        // Append new user data as a separate turn
        let new_content = self.build_new_user_content(data);
        if new_content.as_array().is_some_and(|a| !a.is_empty()) {
            turns.push(json!({
                "role": "user",
                "content": new_content
            }));
        }

        json!(turns)
    }

    /// Convert MessageParts to Interactions API content objects.
    fn convert_message_parts_to_interactions(
        &self,
        parts: &[MessagePart],
    ) -> Vec<serde_json::Value> {
        let mut result = Vec::new();
        let mut prev_thought_sig: Option<String> = None;

        for part in parts {
            match part {
                MessagePart::Text(t) => {
                    result.push(json!({"type": "text", "text": t}));
                    prev_thought_sig = None;
                }
                MessagePart::Part(cp) => {
                    let thought_sig = cp.thought_signature.clone();

                    // Interactions API responses can include internal thought metadata/signatures,
                    // but stateless history input must not contain a standalone `thought` content
                    // object. Preserve any signature by attaching it to the next concrete content
                    // part (text/function_call/image), and otherwise omit the internal thought.
                    if cp.thought.is_some() {
                        if thought_sig.is_some() {
                            prev_thought_sig = thought_sig.clone();
                        }
                    }

                    if let Some(t) = &cp.text
                        && !t.is_empty()
                    {
                        let mut text_obj = json!({"type": "text", "text": t});
                        let effective_sig = thought_sig.clone().or(prev_thought_sig.clone());
                        if let Some(sig) = effective_sig {
                            text_obj["thought_signature"] = json!(sig);
                        }
                        result.push(text_obj);
                        prev_thought_sig = None;
                    }

                    if let Some(fc) = &cp.function_call {
                        let name = fc.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let args = fc.get("arguments").cloned().unwrap_or(json!({}));
                        let id = fc.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        let mut fc_obj = json!({
                            "type": "function_call",
                            "name": name,
                            "arguments": args,
                            "id": id
                        });
                        let effective_sig = thought_sig.clone().or(prev_thought_sig.clone());
                        if let Some(sig) = effective_sig {
                            fc_obj["thought_signature"] = json!(sig);
                        }
                        result.push(fc_obj);
                        prev_thought_sig = None;
                    }

                    if let Some(fr) = &cp.function_response {
                        let name = fr.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let id = fr.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        let response = fr.get("response").cloned().unwrap_or(json!({}));
                        let wrapped_response = if response.is_object() {
                            response
                        } else {
                            json!({ "result": response })
                        };
                        result.push(json!({
                            "type": "function_result",
                            "name": name,
                            "call_id": id,
                            "result": wrapped_response
                        }));
                        prev_thought_sig = None;
                    }

                    if let Some(id) = &cp.inline_data {
                        let mime_type = id.get("mimeType").and_then(|v| v.as_str()).unwrap_or("");
                        let data = id.get("data").and_then(|v| v.as_str()).unwrap_or("");
                        let mut inline_obj = json!({
                            "type": "image",
                            "source": {
                                "inlineData": {
                                    "mimeType": mime_type,
                                    "data": data
                                }
                            }
                        });
                        let effective_sig = thought_sig.clone().or(prev_thought_sig.clone());
                        if let Some(sig) = effective_sig {
                            inline_obj["thought_signature"] = json!(sig);
                        }
                        result.push(inline_obj);
                        prev_thought_sig = None;
                    }
                }
            }
        }

        result
    }

    /// Build the content for new user data only (used for both stateful and stateless).
    fn build_new_user_content(&self, data: &[DataSource]) -> serde_json::Value {
        let mut parts = Vec::new();
        for d in data {
            match d.content_type.as_str() {
                "text/plain" => {
                    parts.push(json!({
                        "type": "text",
                        "text": d.content.as_str().unwrap_or("")
                    }));
                }
                ct if ct.starts_with("image/") || ct.starts_with("application/") => {
                    parts.push(json!({
                        "type": "image",
                        "source": {
                            "inlineData": {
                                "mimeType": ct,
                                "data": d.content.as_str().unwrap_or("")
                            }
                        }
                    }));
                }
                _ => {
                    parts.push(json!({
                        "type": "text",
                        "text": d.content.as_str().unwrap_or("")
                    }));
                }
            }
        }
        json!(parts)
    }

    /// Build tool declarations in Interactions API format:
    /// `[{type: "function", name, description, parameters}]`
    fn build_tools(&self) -> Vec<serde_json::Value> {
        let registry = crate::tools::registry::REGISTRY.lock().unwrap();
        let schemas = registry.get_tool_schemas_gemini();

        // Add native Google Search grounding if brave_search is not registered
        let has_brave = registry.tools.contains_key("brave_search");
        drop(registry);

        let mut tools: Vec<serde_json::Value> = schemas
            .into_iter()
            .map(|s| {
                json!({
                    "type": "function",
                    "name": s["name"],
                    "description": s["description"],
                    "parameters": s["parameters"]
                })
            })
            .collect();

        if !has_brave {
            tools.push(json!({
                "google_search": {}
            }));
        }

        tools
    }

    /// Parse the Interactions API response into text/thought/parts.
    fn parse_response(
        &self,
        res_json: &serde_json::Value,
    ) -> anyhow::Result<(String, String, Vec<MessagePart>, Option<String>)> {
        let mut full_text = String::new();
        let mut thought_text = String::new();
        let mut msg_parts = Vec::new();
        let interaction_id = res_json["id"].as_str().map(|s| s.to_string());

        let outputs = res_json["outputs"].as_array();

        if let Some(outputs) = outputs {
            for output in outputs {
                let output_type = output["type"].as_str().unwrap_or("");
                let thought_sig = output["thought_signature"].as_str().map(|s| s.to_string());

                match output_type {
                    "text" => {
                        let text = output["text"].as_str().unwrap_or("").to_string();
                        full_text.push_str(&text);
                        msg_parts.push(MessagePart::Part(ContentPart {
                            text: Some(text),
                            inline_data: None,
                            function_call: None,
                            function_response: None,
                            thought: None,
                            thought_signature: thought_sig,
                            is_diagnostic: false,
                        }));
                    }
                    "thought" => {
                        let text = output["text"].as_str().unwrap_or("").to_string();
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
                    }
                    "function_call" => {
                        let name = output["name"].as_str().unwrap_or("").to_string();
                        let id = output["id"].as_str().unwrap_or("").to_string();
                        let args = output["arguments"].clone();
                        let mut function_call_map = HashMap::new();
                        function_call_map.insert("name".to_string(), json!(name));
                        function_call_map.insert("arguments".to_string(), args);
                        function_call_map.insert("id".to_string(), json!(id));

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
                    "image" => {
                        // Image output from the model
                        let mime_type = output["mime_type"].as_str().unwrap_or("image/png");
                        let data = output["data"].as_str().unwrap_or("");
                        full_text.push_str(&format!("[Image: {}]", mime_type));

                        let mut inline_map = HashMap::new();
                        inline_map.insert("mimeType".to_string(), json!(mime_type));
                        inline_map.insert("data".to_string(), json!(data));
                        msg_parts.push(MessagePart::Part(ContentPart {
                            text: None,
                            inline_data: Some(inline_map),
                            function_call: None,
                            function_response: None,
                            thought: None,
                            thought_signature: thought_sig,
                            is_diagnostic: false,
                        }));
                    }
                    _ => {
                        // Unknown output type — treat as text if it has text field
                        if let Some(text) = output["text"].as_str() {
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
                        }
                    }
                }
            }
        }

        // Check status for non-completion
        let status = res_json["status"].as_str().unwrap_or("");
        if full_text.is_empty()
            && thought_text.is_empty()
            && msg_parts.is_empty()
            && status != "completed"
        {
            full_text.push_str(&format!("[Interaction status: {}]", status));
        }

        // Extract grounding metadata for citations (Interactions API may include
        // grounding info in the response)
        if let Some(grounding) = res_json.get("grounding_metadata") {
            let mut citations = Vec::new();
            if let Some(chunks) = grounding
                .get("grounding_chunks")
                .or(grounding.get("groundingChunks"))
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

        Ok((full_text, thought_text, msg_parts, interaction_id))
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
        let input = self.build_input(&data);
        let tools = self.build_tools();

        let mut payload = json!({
            "model": self.base.state.model,
            "input": input,
        });

        if let Some(sp) = self.base.state.get_effective_system_prompt() {
            payload["system_instruction"] = json!(sp);
        }

        if self.base.state.tools_enabled && !tools.is_empty() {
            payload["tools"] = json!(tools);
        }

        // If we have a previous interaction ID, chain it for server-side state
        if let Some(ref prev_id) = self.base.state.previous_interaction_id {
            payload["previous_interaction_id"] = json!(prev_id);
        }

        log::debug!(
            "Gemini Interactions Request Payload: {}",
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
                            "Gemini Interactions API rate limit (429) hit. Retrying in {:?}...",
                            backoff
                        );
                        tokio::time::sleep(backoff).await;
                        retries += 1;
                        backoff *= 2;
                        continue;
                    }
                    return Err(anyhow::anyhow!(
                        "Gemini Interactions API request failed: {}",
                        e
                    ));
                }
            }
        };

        let status = res.status();
        let res_json: serde_json::Value = res.json().await.unwrap_or_default();
        log::debug!(
            "Gemini Interactions Response ({}): {}",
            status,
            serde_json::to_string_pretty(&res_json).unwrap_or_default()
        );

        if !status.is_success() {
            let err_msg = res_json["error"]["message"]
                .as_str()
                .unwrap_or("Unknown error")
                .to_string();
            return Err(anyhow::anyhow!(
                "Gemini Interactions API error ({}): {}",
                status,
                err_msg
            ));
        }

        let (full_text, thought_text, msg_parts, interaction_id) =
            self.parse_response(&res_json)?;

        // Store the interaction ID for server-side state chaining
        if let Some(id) = interaction_id {
            self.base.state.previous_interaction_id = Some(id);
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

    async fn send_as_verifier(
        &mut self,
        data: Vec<DataSource>,
        tool_schema: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let input = self.build_input(&data);
        let tool_name = tool_schema["name"].as_str().unwrap_or("verify").to_string();

        let mut payload = json!({
            "model": self.base.state.model,
            "input": input,
            "tools": [{
                "type": "function",
                "name": tool_schema["name"],
                "description": tool_schema["description"],
                "parameters": tool_schema["parameters"]
            }],
            "tool_choice": {
                "type": "function",
                "name": tool_name
            }
        });

        if let Some(sp) = self.base.state.get_effective_system_prompt() {
            payload["system_instruction"] = json!(sp);
        }

        if let Some(ref prev_id) = self.base.state.previous_interaction_id {
            payload["previous_interaction_id"] = json!(prev_id);
        }

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
                            "Gemini Interactions verifier rate limit (429). Retrying in {:?}...",
                            backoff
                        );
                        tokio::time::sleep(backoff).await;
                        retries += 1;
                        backoff *= 2;
                        continue;
                    }
                    return Err(anyhow::anyhow!(
                        "Gemini Interactions API verifier request failed: {}",
                        e
                    ));
                }
            }
        };

        if !res.status().is_success() {
            let res_json: serde_json::Value = res.json().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Gemini Interactions verifier error: {}",
                res_json
            ));
        }

        let res_json: serde_json::Value = res.json().await?;

        // In Interactions API, function call outputs have type "function_call"
        let args = res_json["outputs"]
            .as_array()
            .and_then(|outputs| {
                outputs.iter().find_map(|o| {
                    if o["type"].as_str() == Some("function_call") {
                        Some(o["arguments"].clone())
                    } else {
                        None
                    }
                })
            })
            .ok_or_else(|| {
                anyhow::anyhow!("No function_call output in Gemini Interactions response")
            })?;

        Ok(args)
    }
}
