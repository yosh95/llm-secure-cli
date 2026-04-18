use crate::config::CONFIG_MANAGER;
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
    ) -> anyhow::Result<(Option<String>, Option<String>)>;

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
        initial_model_alias: &str,
        spec: ProviderSpec,
        stdout: bool,
        render_markdown: bool,
    ) -> Self {
        let config_section = spec.config_section.clone();
        let api_key = CONFIG_MANAGER.get_api_key(&config_section);
        let model_config = CONFIG_MANAGER.get_model_config(&config_section, initial_model_alias);

        let model_name = model_config
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or(initial_model_alias)
            .to_string();

        // Load system prompt from config if available
        let system_prompt = {
            let config = CONFIG_MANAGER.get_config();
            config
                .providers
                .get(&config_section)
                .and_then(|p| p.extra.get("system_prompt"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        };

        let state = ClientState {
            model: model_name,
            provider: config_section.clone(),
            conversation: Vec::new(),
            tools_enabled: true,
            system_prompt_enabled: true,
            system_prompt,
            stdout,
            render_markdown,
            live_debug: false,
        };

        Self {
            state,
            config_section,
            api_key,
        }
    }
}
