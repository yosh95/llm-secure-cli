use crate::config::ConfigManager;
use crate::llm::models::{ClientState, DataSource, Message};
use async_trait::async_trait;

pub struct ProviderSpec {
    pub api_key_name: String,
    pub config_section: String,
    pub pdf_as_base64: bool,
}

#[async_trait]
pub trait LlmClient: Send + Sync {
    fn get_state(&self) -> &ClientState;
    fn get_state_mut(&mut self) -> &mut ClientState;
    fn get_config_section(&self) -> &str;

    async fn send(
        &mut self,
        data: Vec<DataSource>,
        tool_schemas: Vec<serde_json::Value>,
    ) -> anyhow::Result<(Option<String>, Option<String>)>;

    /// Send a request specifically for verification purposes, forcing a structured tool call response.
    async fn send_as_verifier(
        &mut self,
        data: Vec<DataSource>,
        tool_schema: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value>;

    fn load_session(&mut self, path: &str) -> anyhow::Result<()> {
        let file = std::fs::File::open(path)?;
        let conversation: Vec<crate::llm::models::Message> = serde_json::from_reader(file)?;
        self.get_state_mut().conversation = conversation;
        Ok(())
    }

    fn save_session(&self, path: &str) -> anyhow::Result<()> {
        let file = std::fs::File::create(path)?;
        serde_json::to_writer_pretty(file, &self.get_state().conversation)?;
        Ok(())
    }

    fn get_display_name(&self) -> String {
        format!("LLM ({})", self.get_state().model)
    }

    fn should_send_pdf_as_base64(&self) -> bool;

    fn update_history(&mut self, data: &[DataSource], model_msg: Message) {
        let mut user_parts = Vec::new();
        for d in data {
            if d.content_type == "text/plain" {
                if let Some(text) = d.content.as_str() {
                    user_parts.push(crate::llm::models::MessagePart::Text(text.to_string()));
                }
            } else {
                let mut inline = std::collections::HashMap::new();
                inline.insert("mimeType".to_string(), serde_json::json!(d.content_type));
                inline.insert("data".to_string(), d.content.clone());
                if let Some(filename) = d.metadata.get("filename") {
                    inline.insert("filename".to_string(), filename.clone());
                }
                user_parts.push(crate::llm::models::MessagePart::Part(
                    crate::llm::models::ContentPart {
                        text: None,
                        inline_data: Some(inline),
                        function_call: None,
                        function_response: None,
                        thought: None,
                        thought_signature: None,
                        is_diagnostic: false,
                    },
                ));
            }
        }
        if !user_parts.is_empty() {
            self.get_state_mut()
                .conversation
                .push(crate::llm::models::Message {
                    role: crate::llm::models::Role::User,
                    parts: user_parts,
                });
        }
        self.get_state_mut().conversation.push(model_msg);
    }
}

pub struct BaseLlmClientData {
    pub state: ClientState,
    pub config_section: String,
    pub api_key: Option<String>,
}

impl BaseLlmClientData {
    pub fn new(
        config_manager: &ConfigManager,
        initial_model_alias: &str,
        spec: ProviderSpec,
        stdout: bool,
        raw: bool,
    ) -> Self {
        let config_section = spec.config_section.clone();
        let api_key = config_manager.get_api_key(&config_section);
        let model_config = config_manager.get_model_config(&config_section, initial_model_alias);

        let model_name = model_config
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or(initial_model_alias)
            .to_string();

        // Load system prompt from config if available
        let system_prompt = {
            let config = config_manager.get_config();
            let provider_cfg = config.providers.get(&config_section);
            provider_cfg
                .and_then(|p| p.system_prompt.clone())
                .or_else(|| {
                    provider_cfg
                        .and_then(|p| p.extra.get("system_prompt"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
        };

        let tools_enabled = model_config
            .get("tools")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let state = ClientState {
            model: model_name,
            provider: config_section.clone(),
            conversation: Vec::new(),
            tools_enabled,
            system_prompt_enabled: true,
            system_prompt,
            stdout,
            render_markdown: !raw,
            live_debug: false,
            previous_interaction_id: None,
        };

        Self {
            state,
            config_section,
            api_key,
        }
    }
}

/// Creates a reqwest client with timeout settings from the global config.
pub fn create_http_client(config_manager: &ConfigManager) -> reqwest::Client {
    let config = config_manager.get_config();
    let timeout_secs = config.general.request_timeout;

    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
        .expect("Failed to create reqwest client")
}
