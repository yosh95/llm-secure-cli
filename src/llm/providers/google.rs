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

        // Preserve message boundaries. Although `Role::Tool` is represented as a
        // `user` turn in the Interactions API, it must remain a separate turn from
        // a normal user message. Merging adjacent `user` turns can produce content
        // like `[function_result, text]` in a single turn, which the Interactions
        // API rejects with `invalid_request` for tool-call histories.
        for m in &self.base.state.conversation {
            let role_str = match m.role {
                Role::User | Role::Tool => "user",
                Role::Assistant | Role::Model => "model",
                Role::System => continue,
            };

            let parts = self.convert_message_parts_to_interactions(&m.parts);
            if !parts.is_empty() {
                turns.push(json!({
                    "role": role_str,
                    "content": parts
                }));
            }
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

        for part in parts {
            match part {
                MessagePart::Text(t) => {
                    result.push(json!({"type": "text", "text": t}));
                }
                MessagePart::Part(cp) => {
                    // Gemini 3 models REQUIRE `thought_signature` to round-trip on
                    // function_call parts in the current turn — otherwise the API
                    // returns 400 ("Function call is missing a thought_signature").
                    //
                    // Interactions API returns dedicated `thought` parts (often
                    // empty-text, signature-only). The official client-side history
                    // example simply re-appends `interaction.outputs` verbatim, so we
                    // emit a `thought` content object for these parts and preserve
                    // the signature on it.
                    if cp.thought.is_some() {
                        let thought_text = cp.thought.clone().unwrap_or_default();
                        let mut t_obj = json!({"type": "thought"});
                        if !thought_text.is_empty() {
                            t_obj["text"] = json!(thought_text);
                        }
                        if let Some(sig) = cp.thought_signature.clone() {
                            // Interactions API uses the short field name `signature`
                            // on dedicated thought parts.
                            t_obj["signature"] = json!(sig);
                        }
                        result.push(t_obj);
                        continue;
                    }

                    if let Some(t) = &cp.text
                        && !t.is_empty()
                    {
                        result.push(json!({"type": "text", "text": t}));
                    }

                    if let Some(fc) = &cp.function_call {
                        let name = fc.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let args = fc.get("arguments").cloned().unwrap_or(json!({}));
                        let id = fc.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        result.push(json!({
                            "type": "function_call",
                            "name": name,
                            "arguments": args,
                            "id": id
                        }));
                    }

                    if let Some(fr) = &cp.function_response {
                        let name = fr.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let id = fr.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        let response = fr.get("response").cloned().unwrap_or(json!({}));
                        // Interactions API requires `result` to be an array of
                        // FunctionResultSubcontent objects (e.g. [{type:"text", text:"..."}])
                        // or a plain string — not a bare JSON object.
                        let result_array = if response.is_string() {
                            // Already a string — wrap as a single text subcontent.
                            json!([{"type": "text", "text": response.as_str().unwrap_or("")}])
                        } else {
                            // Serialise the object to a JSON string and wrap it.
                            let text = serde_json::to_string(&response).unwrap_or_default();
                            json!([{"type": "text", "text": text}])
                        };
                        result.push(json!({
                            "type": "function_result",
                            "name": name,
                            "call_id": id,
                            "result": result_array
                        }));
                    }

                    if let Some(id) = &cp.inline_data {
                        let mime_type = id.get("mimeType").and_then(|v| v.as_str()).unwrap_or("");
                        let data = id.get("data").and_then(|v| v.as_str()).unwrap_or("");
                        let part_type = if mime_type == "application/pdf" {
                            "document"
                        } else if mime_type.starts_with("audio/") {
                            "audio"
                        } else if mime_type.starts_with("video/") {
                            "video"
                        } else {
                            "image"
                        };
                        result.push(json!({
                            "type": part_type,
                            "mime_type": mime_type,
                            "data": data
                        }));
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
                ct if ct.starts_with("image/")
                    || ct.starts_with("audio/")
                    || ct.starts_with("video/")
                    || ct.starts_with("application/") =>
                {
                    let part_type = if ct.starts_with("application/") {
                        "document"
                    } else if ct.starts_with("audio/") {
                        "audio"
                    } else if ct.starts_with("video/") {
                        "video"
                    } else {
                        "image"
                    };
                    parts.push(json!({
                        "type": part_type,
                        "mime_type": ct,
                        "data": d.content.as_str().unwrap_or("")
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
    fn build_tools(&self, tool_schemas: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
        // Add native Google Search grounding if brave_search is not registered
        let has_brave = tool_schemas.iter().any(|s| {
            s.get("name")
                .and_then(|v| v.as_str())
                .map(|s| s == "brave_search")
                .unwrap_or(false)
        });

        let mut tools: Vec<serde_json::Value> = tool_schemas
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
                "type": "google_search"
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
        let mut search_thought_text = String::new();
        let mut msg_parts = Vec::new();
        let interaction_id = res_json["id"].as_str().map(|s| s.to_string());

        let outputs = res_json["outputs"].as_array();

        if let Some(outputs) = outputs {
            for output in outputs {
                let output_type = output["type"].as_str().unwrap_or("");
                // Interactions API may use either `thought_signature` (long form, on
                // text/function_call parts) or `signature` (short form, on dedicated
                // `thought` parts and on tool call parts like `url_context_call`).
                // Read whichever is present so we can faithfully echo it back in the
                // next request — Gemini 3 requires the signature to round-trip on
                // function_call parts or it returns 400.
                let thought_sig = output["thought_signature"]
                    .as_str()
                    .or_else(|| output["signature"].as_str())
                    .map(|s| s.to_string());

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
                        search_thought_text.push_str(&text);
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
            && search_thought_text.is_empty()
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

        Ok((full_text, search_thought_text, msg_parts, interaction_id))
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
        let input = self.build_input(&data);
        let tools = self.build_tools(tool_schemas);

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
    fn build_input_preserves_thought_parts_in_history() {
        // Gemini 3 requires thought signatures to round-trip with function_call
        // parts. We preserve standalone `thought` parts in the re-submitted history
        // so any signature attached to them is not lost.
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
                        thought_signature: Some("sig-thought".to_string()),
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
        // thought parts ARE preserved (with signature) so Gemini 3 validation passes.
        assert!(serialized.contains("\"type\":\"thought\""));
        assert!(serialized.contains("\"signature\":\"sig-thought\""));
        assert!(serialized.contains("\"type\":\"function_call\""));
        assert!(serialized.contains("\"type\":\"text\""));
    }
    #[test]
    fn build_input_carries_thought_signature_to_next_concrete_part() {
        // `thought` parts carry the signature internally (as `signature`).
        // We NO LONGER attach `thought_signature` to subsequent non-thought
        // parts (function_call, text, etc.) because the Interactions API
        // rejects the unknown parameter on anything other than `thought`.
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

        // Thought part is emitted with `signature`.
        assert_eq!(model_content.len(), 2);
        assert_eq!(model_content[0]["type"], "thought");
        assert_eq!(model_content[0]["signature"], "sig-1");

        // function_call must NOT carry the signature (API rejects it there).
        assert_eq!(model_content[1]["type"], "function_call");
        assert!(model_content[1].get("thought_signature").is_none());
    }

    #[test]
    fn build_input_keeps_tool_result_and_user_text_in_separate_turns() {
        let mut fr = HashMap::new();
        fr.insert("name".to_string(), json!("execute_command"));
        fr.insert("id".to_string(), json!("call-1"));
        fr.insert(
            "response".to_string(),
            json!({"exit_code": 0, "stdout": "ok", "stderr": ""}),
        );

        let client = test_client(vec![
            Message {
                role: Role::Tool,
                parts: vec![MessagePart::Part(ContentPart {
                    text: None,
                    inline_data: None,
                    function_call: None,
                    function_response: Some(fr),
                    thought: None,
                    thought_signature: None,
                    is_diagnostic: false,
                })],
            },
            Message {
                role: Role::User,
                parts: vec![MessagePart::Text("git commit".to_string())],
            },
        ]);

        let input = client.build_input(&[]);
        let turns = input.as_array().unwrap();

        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0]["role"], "user");
        assert_eq!(turns[0]["content"][0]["type"], "function_result");
        assert_eq!(turns[1]["role"], "user");
        assert_eq!(turns[1]["content"][0]["type"], "text");
    }

    #[test]
    fn build_input_function_result_uses_array_format() {
        // Interactions API requires `result` to be an array of FunctionResultSubcontent
        // (e.g. [{type:"text", text:"..."}]), NOT a bare JSON object.
        let mut fr = HashMap::new();
        fr.insert("name".to_string(), json!("execute_command"));
        fr.insert("id".to_string(), json!("call-42"));
        fr.insert(
            "response".to_string(),
            json!({"exit_code": 0, "stdout": "On branch main\n", "stderr": ""}),
        );

        let client = test_client(vec![Message {
            role: Role::Tool,
            parts: vec![MessagePart::Part(ContentPart {
                text: None,
                inline_data: None,
                function_call: None,
                function_response: Some(fr),
                thought: None,
                thought_signature: None,
                is_diagnostic: false,
            })],
        }]);

        let input = client.build_input(&[]);
        let turns = input.as_array().unwrap();
        let fr_content = &turns[0]["content"][0];

        assert_eq!(fr_content["type"], "function_result");
        assert_eq!(fr_content["call_id"], "call-42");

        // `result` must be an array, not an object
        let result = &fr_content["result"];
        assert!(
            result.is_array(),
            "result must be an array, got: {}",
            result
        );

        let result_arr = result.as_array().unwrap();
        assert_eq!(result_arr.len(), 1);
        assert_eq!(result_arr[0]["type"], "text");
        // The object was serialised into the text field
        let text = result_arr[0]["text"].as_str().unwrap();
        assert!(
            text.contains("exit_code"),
            "serialised JSON should contain field names"
        );
    }

    #[test]
    fn build_input_thought_without_sig_is_preserved_with_text() {
        // Thought parts with no signature are still preserved in history, but
        // emitted with their text and no `signature` field. (Gemini 3 only
        // strictly requires signatures on function_call parts of the *current*
        // turn, but preserving thought-text is harmless and matches the
        // documented behaviour of re-appending `interaction.outputs` verbatim.)
        let mut fc = HashMap::new();
        fc.insert("name".to_string(), json!("execute_command"));
        fc.insert("id".to_string(), json!("call-2"));
        fc.insert("arguments".to_string(), json!({"command": "ls"}));

        let client = test_client(vec![Message {
            role: Role::Model,
            parts: vec![
                // thought with no signature
                MessagePart::Part(ContentPart {
                    text: None,
                    inline_data: None,
                    function_call: None,
                    function_response: None,
                    thought: Some("internal reasoning".to_string()),
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
        }]);

        let input = client.build_input(&[]);
        let serialized = serde_json::to_string(&input).unwrap();

        // thought type IS now present in the serialised history (with the text)
        assert!(serialized.contains("\"type\":\"thought\""));
        assert!(serialized.contains("\"text\":\"internal reasoning\""));
        // function_call must still be present
        assert!(serialized.contains("\"type\":\"function_call\""));
        // No signature field on the thought (it had none)
        let model_content = input[0]["content"].as_array().unwrap();
        assert_eq!(model_content[0]["type"], "thought");
        assert!(model_content[0].get("signature").is_none());
    }

    #[test]
    fn build_input_multimodal_data_uses_correct_structure() {
        let client = test_client(vec![]);
        let data = vec![
            DataSource {
                content: json!("base64pdf"),
                content_type: "application/pdf".to_string(),
                is_file_or_url: true,
                metadata: HashMap::new(),
            },
            DataSource {
                content: json!("base64img"),
                content_type: "image/png".to_string(),
                is_file_or_url: true,
                metadata: HashMap::new(),
            },
        ];

        let input = client.build_input(&data);
        let turns = input.as_array().unwrap();
        let contents = turns[0]["content"].as_array().unwrap();

        assert_eq!(contents.len(), 2);

        // PDF part
        assert_eq!(contents[0]["type"], "document");
        assert_eq!(contents[0]["mime_type"], "application/pdf");
        assert_eq!(contents[0]["data"], "base64pdf");
        assert!(contents[0].get("inline_data").is_none());
        assert!(contents[0].get("source").is_none());

        // Image part
        assert_eq!(contents[1]["type"], "image");
        assert_eq!(contents[1]["mime_type"], "image/png");
        assert_eq!(contents[1]["data"], "base64img");
        assert!(contents[1].get("inline_data").is_none());
        assert!(contents[1].get("source").is_none());
    }
}
