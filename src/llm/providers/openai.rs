use crate::config::ConfigManager;
use crate::llm::base::{BaseLlmClientData, LlmClient, ProviderSpec};
use crate::llm::models::{ClientState, ContentPart, DataSource, Message, MessagePart, Role};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

// Note: This static CLIENT is initialized without config-based timeout for now.
// For full DI, it should be passed from AppContext.
static CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .build()
        .expect("Failed to create reqwest client")
});

pub struct OpenAiClient {
    pub base: BaseLlmClientData,
    pub api_url: String,
    image_generation_enabled: bool,
    prompt_cache_enabled: bool,
    prompt_cache_retention: Option<String>,
}

impl OpenAiClient {
    pub fn new(config_manager: &ConfigManager, model: &str, stdout: bool, raw: bool) -> Self {
        let spec = ProviderSpec {
            api_key_name: "api_key".to_string(),
            config_section: "openai".to_string(),
            pdf_as_base64: true,
        };
        let model_config = config_manager.get_model_config("openai", model);
        let base = BaseLlmClientData::new(config_manager, model, spec, stdout, raw);
        let model_lc = base.state.model.to_lowercase();
        let image_generation_enabled = model_config
            .get("image_generation")
            .and_then(|v| v.as_bool())
            .unwrap_or_else(|| {
                model.eq_ignore_ascii_case("image")
                    || model_lc.contains("gpt-image")
                    || model_lc.contains("dall-e")
                    || model_lc.contains("image")
            });
        let prompt_cache_enabled = model_config
            .get("prompt_cache")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let prompt_cache_retention = model_config
            .get("prompt_cache_retention")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != "default" && *s != "in_memory")
            .map(ToOwned::to_owned);

        Self {
            base,
            api_url: "https://api.openai.com/v1/responses".to_string(),
            image_generation_enabled,
            prompt_cache_enabled,
            prompt_cache_retention,
        }
    }

    fn data_url(mime_type: &str, b64_data: &str) -> String {
        format!("data:{};base64,{}", mime_type, b64_data)
    }

    fn filename_from_data_source(d: &DataSource, fallback: &str) -> String {
        d.metadata
            .get("filename")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(fallback)
            .to_string()
    }

    fn data_source_to_input_part(d: &DataSource) -> Option<Value> {
        let content = d.content.as_str().unwrap_or("");
        if content.is_empty() {
            return None;
        }

        if d.content_type == "text/plain" {
            Some(json!({
                "type": "input_text",
                "text": content
            }))
        } else if d.content_type.starts_with("image/") {
            Some(json!({
                "type": "input_image",
                "image_url": Self::data_url(&d.content_type, content)
            }))
        } else if d.content_type == "application/pdf" {
            Some(json!({
                "type": "input_file",
                "filename": Self::filename_from_data_source(d, "document.pdf"),
                "file_data": Self::data_url("application/pdf", content)
            }))
        } else if d.content_type.starts_with("audio/") || d.content_type.starts_with("video/") {
            // Audio/video are intentionally not supported for OpenAI Responses in this CLI.
            Some(json!({
                "type": "input_text",
                "text": format!(
                    "[Unsupported media omitted: {}. Audio/video input is not enabled for the OpenAI Responses provider.]",
                    d.content_type
                )
            }))
        } else {
            Some(json!({
                "type": "input_file",
                "filename": Self::filename_from_data_source(d, "file.bin"),
                "file_data": Self::data_url(&d.content_type, content)
            }))
        }
    }

    fn inline_data_to_input_part(inline: &HashMap<String, Value>) -> Option<Value> {
        let mime = inline
            .get("mimeType")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let data = inline.get("data").and_then(|v| v.as_str()).unwrap_or("");
        if mime.is_empty() || data.is_empty() {
            return None;
        }

        if mime.starts_with("image/") {
            Some(json!({
                "type": "input_image",
                "image_url": Self::data_url(mime, data)
            }))
        } else if mime == "application/pdf" {
            Some(json!({
                "type": "input_file",
                "filename": inline
                    .get("filename")
                    .and_then(|v| v.as_str())
                    .unwrap_or("document.pdf"),
                "file_data": Self::data_url(mime, data)
            }))
        } else if mime.starts_with("audio/") || mime.starts_with("video/") {
            Some(json!({
                "type": "input_text",
                "text": format!("[Unsupported media omitted from history: {}]", mime)
            }))
        } else {
            Some(json!({
                "type": "input_file",
                "filename": inline
                    .get("filename")
                    .and_then(|v| v.as_str())
                    .unwrap_or("file.bin"),
                "file_data": Self::data_url(mime, data)
            }))
        }
    }

    fn message_to_response_items(&self, m: &Message) -> Vec<Value> {
        let mut items = Vec::new();

        match m.role {
            Role::System => {
                // System prompt is sent via `instructions` instead of as an input item.
            }
            Role::Tool => {
                for part in &m.parts {
                    if let MessagePart::Part(cp) = part
                        && let Some(fr) = &cp.function_response
                    {
                        let call_id = fr.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        if call_id.is_empty() {
                            continue;
                        }
                        let response = fr.get("response").cloned().unwrap_or(json!(""));
                        let output = response
                            .as_str()
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| response.to_string());

                        items.push(json!({
                            "type": "function_call_output",
                            "call_id": call_id,
                            "output": output
                        }));
                    }
                }
            }
            Role::User | Role::Assistant | Role::Model => {
                let role = match m.role {
                    Role::User => "user",
                    Role::Assistant | Role::Model => "assistant",
                    _ => unreachable!(),
                };

                let mut content_parts = Vec::new();

                for part in &m.parts {
                    match part {
                        MessagePart::Text(t) => {
                            if !t.is_empty() {
                                let part_type = if role == "assistant" {
                                    "output_text"
                                } else {
                                    "input_text"
                                };
                                content_parts.push(json!({
                                    "type": part_type,
                                    "text": t
                                }));
                            }
                        }
                        MessagePart::Part(cp) => {
                            if let Some(t) = &cp.text
                                && !t.is_empty()
                            {
                                let part_type = if role == "assistant" {
                                    "output_text"
                                } else {
                                    "input_text"
                                };
                                content_parts.push(json!({
                                    "type": part_type,
                                    "text": t
                                }));
                            }

                            // Preserve user-provided images/files/PDFs. Assistant-generated images are
                            // summarized as text to avoid re-uploading outputs as assistant input.
                            if role == "user"
                                && let Some(inline) = &cp.inline_data
                                && let Some(part) = Self::inline_data_to_input_part(inline)
                            {
                                content_parts.push(part);
                            }

                            if let Some(fc) = &cp.function_call {
                                let call_id = fc.get("id").and_then(|v| v.as_str()).unwrap_or("");
                                let name = fc.get("name").and_then(|v| v.as_str()).unwrap_or("");
                                if !call_id.is_empty() && !name.is_empty() {
                                    let arguments = fc
                                        .get("arguments")
                                        .map(|v| {
                                            v.as_str()
                                                .map(|s| s.to_string())
                                                .unwrap_or_else(|| v.to_string())
                                        })
                                        .unwrap_or_else(|| "{}".to_string());
                                    items.push(json!({
                                        "type": "function_call",
                                        "call_id": call_id,
                                        "name": name,
                                        "arguments": arguments
                                    }));
                                }
                            }
                        }
                    }
                }

                if !content_parts.is_empty() {
                    items.push(json!({
                        "type": "message",
                        "role": role,
                        "content": content_parts
                    }));
                }
            }
        }

        items
    }

    fn build_responses_input(&self, data: &[DataSource]) -> Vec<Value> {
        let mut input = Vec::new();

        for m in &self.base.state.conversation {
            input.extend(self.message_to_response_items(m));
        }

        let new_parts = data
            .iter()
            .filter_map(Self::data_source_to_input_part)
            .collect::<Vec<_>>();

        if !new_parts.is_empty() {
            input.push(json!({
                "type": "message",
                "role": "user",
                "content": new_parts
            }));
        }

        input
    }

    fn build_responses_tools(&self, tool_schemas: Vec<Value>) -> Vec<Value> {
        let mut tools = Vec::new();

        if self.base.state.tools_enabled {
            // Include native web_search tool if brave_search is not registered
            let has_brave = tool_schemas.iter().any(|s| {
                s.get("name")
                    .and_then(|v| v.as_str())
                    .map(|s| s == "brave_search")
                    .unwrap_or(false)
            });

            if !has_brave {
                tools.push(json!({
                    "type": "web_search"
                }));
            }

            tools.extend(tool_schemas.into_iter().map(|s| {
                json!({
                    "type": "function",
                    "name": s.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                    "description": s.get("description").and_then(|v| v.as_str()).unwrap_or(""),
                    "parameters": s.get("parameters").cloned().unwrap_or_else(|| json!({"type":"object","properties":{}}))
                })
            }));
        }

        if self.image_generation_enabled {
            tools.push(json!({
                "type": "image_generation"
            }));
        }

        tools
    }

    fn prompt_cache_key_for(&self, instructions: Option<&str>, tools: &[Value]) -> String {
        // Keep this key stable for requests that share the same static prompt/tool prefix.
        // Do not include user input or conversation text; those change every turn and would
        // reduce server-side prompt cache reuse.
        let mut hasher = Sha256::new();
        hasher.update(b"llsc-openai-responses-v1\0");
        hasher.update(self.base.state.model.as_bytes());
        hasher.update(b"\0");
        hasher.update(if self.base.state.tools_enabled {
            b"tools:on".as_slice()
        } else {
            b"tools:off".as_slice()
        });
        hasher.update(b"\0");
        hasher.update(if self.image_generation_enabled {
            b"image:on".as_slice()
        } else {
            b"image:off".as_slice()
        });
        hasher.update(b"\0");
        if let Some(instructions) = instructions {
            hasher.update(instructions.as_bytes());
        }
        hasher.update(b"\0");
        let tools_json = serde_json::to_string(tools).unwrap_or_default();
        hasher.update(tools_json.as_bytes());

        let digest = hex::encode(hasher.finalize());
        format!("llsc-openai-{}", &digest[..32])
    }

    fn apply_prompt_cache(&self, payload: &mut Value, instructions: Option<&str>, tools: &[Value]) {
        if !self.prompt_cache_enabled {
            return;
        }

        payload["prompt_cache_key"] = json!(self.prompt_cache_key_for(instructions, tools));

        if let Some(retention) = &self.prompt_cache_retention {
            payload["prompt_cache_retention"] = json!(retention);
        }
    }

    fn build_payload(&self, data: &[DataSource], tool_schemas: Vec<Value>) -> Value {
        let instructions = self.base.state.get_effective_system_prompt();
        let tools = self.build_responses_tools(tool_schemas);
        let mut payload = json!({
            "model": self.base.state.model,
            "input": self.build_responses_input(data),
            "store": false
        });

        if let Some(instructions) = &instructions {
            payload["instructions"] = json!(instructions);
        }

        if !tools.is_empty() {
            payload["tools"] = json!(tools);
        }

        self.apply_prompt_cache(&mut payload, instructions.as_deref(), &tools);
        payload
    }

    async fn post_responses(&self, payload: Value, context: &str) -> anyhow::Result<Value> {
        log::debug!(
            "OpenAI Responses Request Payload: {}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );

        let mut retries = 0;
        let max_retries = 3;
        let mut backoff = std::time::Duration::from_secs(2);

        let res = loop {
            let mut req = CLIENT.post(&self.api_url);
            if let Some(key) = &self.base.api_key {
                req = req.header("Authorization", format!("Bearer {}", key));
            }
            let res_result = req.json(&payload).send().await;

            match res_result {
                Ok(r) => break r,
                Err(e) => {
                    let status_code = e.status().map(|s| s.as_u16()).unwrap_or(0);
                    let should_retry = status_code == 429 || status_code >= 500;

                    if should_retry && retries < max_retries {
                        log::warn!(
                            "OpenAI Responses API error ({}) in {}. Retrying in {:?}...",
                            e,
                            context,
                            backoff
                        );
                        tokio::time::sleep(backoff).await;
                        retries += 1;
                        backoff *= 2;
                        continue;
                    }
                    return Err(anyhow::anyhow!(
                        "OpenAI Responses API request failed in {}: {}",
                        context,
                        e
                    ));
                }
            }
        };

        let status = res.status();
        let res_json: Value = res.json().await.unwrap_or_default();
        log::debug!(
            "OpenAI Responses Response ({}): {}",
            status,
            serde_json::to_string_pretty(&res_json).unwrap_or_default()
        );

        if let Some(usage) = res_json.get("usage") {
            let input_tokens = usage
                .get("input_tokens")
                .or_else(|| usage.get("prompt_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let cached_tokens = usage
                .get("input_tokens_details")
                .or_else(|| usage.get("prompt_tokens_details"))
                .and_then(|v| v.get("cached_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            if input_tokens > 0 {
                log::debug!(
                    "OpenAI Prompt Cache usage in {}: cached_tokens={} / input_tokens={} ({:.1}%)",
                    context,
                    cached_tokens,
                    input_tokens,
                    (cached_tokens as f64 * 100.0) / input_tokens as f64
                );
            }
        }

        if !status.is_success() {
            if let Some(err) = res_json.get("error") {
                return Err(anyhow::anyhow!(
                    "OpenAI Responses API error ({}) in {}: {}",
                    status,
                    context,
                    err
                ));
            }
            return Err(anyhow::anyhow!(
                "OpenAI Responses API error ({}) in {}: {}",
                status,
                context,
                res_json
            ));
        }

        Ok(res_json)
    }

    fn parse_output_text_from_content(
        content: &[Value],
        full_text: &mut String,
        model_parts: &mut Vec<MessagePart>,
    ) {
        for part in content {
            let part_type = part.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if matches!(part_type, "output_text" | "text" | "input_text")
                && let Some(t) = part.get("text").and_then(|v| v.as_str())
            {
                let mut block_text = t.to_string();

                // Extract citations if present (OpenAI Responses API format)
                if let Some(citations) = part.get("citations").and_then(|v| v.as_array()) {
                    let mut citations_list = Vec::new();
                    for (i, citation) in citations.iter().enumerate() {
                        let title = citation
                            .get("title")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Source");
                        let url = citation.get("url").and_then(|v| v.as_str()).unwrap_or("");
                        if !url.is_empty() {
                            citations_list.push(format!("[{}] [{}]({})", i + 1, title, url));
                        }
                    }
                    if !citations_list.is_empty() {
                        block_text.push_str("\n\n**Sources:**\n");
                        block_text.push_str(&citations_list.join("\n"));
                    }
                }

                full_text.push_str(&block_text);
                model_parts.push(MessagePart::Text(block_text));
            }
        }
    }

    fn parse_responses_output(&self, res_json: &Value) -> (String, Vec<MessagePart>) {
        let mut full_text = String::new();
        let mut model_parts = Vec::new();

        if let Some(t) = res_json.get("output_text").and_then(|v| v.as_str())
            && !t.is_empty()
        {
            full_text.push_str(t);
            model_parts.push(MessagePart::Text(t.to_string()));
        }

        if let Some(output) = res_json.get("output").and_then(|v| v.as_array()) {
            for item in output {
                let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match item_type {
                    "message" => {
                        if let Some(content) = item.get("content").and_then(|v| v.as_array()) {
                            // Avoid duplicate text when output_text already contained the aggregate.
                            if full_text.is_empty() {
                                Self::parse_output_text_from_content(
                                    content,
                                    &mut full_text,
                                    &mut model_parts,
                                );
                            }
                        }
                    }
                    "function_call" => {
                        let call_id = item
                            .get("call_id")
                            .or_else(|| item.get("id"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let name = item
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let args_str = item
                            .get("arguments")
                            .and_then(|v| v.as_str())
                            .unwrap_or("{}")
                            .to_string();
                        let args: HashMap<String, Value> =
                            serde_json::from_str(&args_str).unwrap_or_default();

                        let mut function_call = HashMap::new();
                        function_call.insert("name".to_string(), json!(name));
                        function_call.insert("arguments".to_string(), json!(args));
                        function_call.insert("id".to_string(), json!(call_id));

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
                    "image_generation_call" => {
                        if let Some(b64) = item.get("result").and_then(|v| v.as_str()) {
                            let mime_type = item
                                .get("mime_type")
                                .or_else(|| item.get("mimeType"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("image/png")
                                .to_string();
                            if full_text.is_empty() {
                                full_text.push_str("[Generated image]");
                            } else {
                                full_text.push_str("\n[Generated image]");
                            }

                            let mut inline_data = HashMap::new();
                            inline_data.insert("mimeType".to_string(), json!(mime_type));
                            inline_data.insert("data".to_string(), json!(b64));

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
                    }
                    _ => {}
                }
            }
        }

        (full_text, model_parts)
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
    fn should_send_pdf_as_base64(&self) -> bool {
        true
    }

    async fn send(
        &mut self,
        data: Vec<DataSource>,
        tool_schemas: Vec<Value>,
    ) -> anyhow::Result<(Option<String>, Option<String>)> {
        let payload = self.build_payload(&data, tool_schemas);
        let res_json = self.post_responses(payload, "chat").await?;

        let (full_text, mut model_parts) = self.parse_responses_output(&res_json);
        if model_parts.is_empty() && full_text.is_empty() {
            model_parts.push(MessagePart::Text("".to_string()));
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

    async fn send_as_verifier(
        &mut self,
        data: Vec<DataSource>,
        tool_schema: Value,
    ) -> anyhow::Result<Value> {
        let tool_name = tool_schema["name"].as_str().unwrap_or("verify").to_string();
        let verifier_tool = json!({
            "type": "function",
            "name": tool_schema.get("name").and_then(|v| v.as_str()).unwrap_or("verify"),
            "description": tool_schema.get("description").and_then(|v| v.as_str()).unwrap_or(""),
            "parameters": tool_schema.get("parameters").cloned().unwrap_or_else(|| json!({"type":"object","properties":{}}))
        });
        let verifier_tools = vec![verifier_tool];
        let instructions = self.base.state.get_effective_system_prompt();
        let mut payload = json!({
            "model": self.base.state.model,
            "input": self.build_responses_input(&data),
            "tools": verifier_tools,
            "tool_choice": {
                "type": "function",
                "name": tool_name
            },
            "store": false
        });

        if let Some(instructions) = &instructions {
            payload["instructions"] = json!(instructions);
        }

        self.apply_prompt_cache(
            &mut payload,
            instructions.as_deref(),
            verifier_tools.as_slice(),
        );

        let res_json = self.post_responses(payload, "verifier").await?;
        let output = res_json
            .get("output")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("No output in OpenAI Responses verifier response"))?;

        for item in output {
            if item.get("type").and_then(|v| v.as_str()) == Some("function_call") {
                let args_str = item
                    .get("arguments")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "No function call arguments in OpenAI Responses verifier response"
                        )
                    })?;
                let args: Value = serde_json::from_str(args_str)?;
                return Ok(args);
            }
        }

        Err(anyhow::anyhow!(
            "No function_call item in OpenAI Responses verifier response"
        ))
    }
}
