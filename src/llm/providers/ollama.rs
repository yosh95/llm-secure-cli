use crate::config::ConfigManager;
use crate::llm::base::LlmClient;
use crate::llm::models::{ClientState, DataSource};
use crate::llm::providers::openai_compatible::OpenAiCompatibleClient;
use async_trait::async_trait;
use serde_json::Value;

pub struct OllamaClient {
    inner: OpenAiCompatibleClient,
}

impl OllamaClient {
    pub fn new(
        config_manager: &ConfigManager,
        provider_name: &str,
        api_url: &str,
        api_key: &str,
        model: &str,
        stdout: bool,
        raw: bool,
    ) -> anyhow::Result<Self> {
        // Ollama uses OpenAI compatibility mode natively on /v1/chat/completions
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
impl LlmClient for OllamaClient {
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
        // Ollama doesn't natively support PDF as multimodal input yet.
        // Returning false ensures that text is extracted from the PDF and sent as text.
        false
    }

    async fn send(
        &mut self,
        data: Vec<DataSource>,
        tool_schemas: Vec<Value>,
    ) -> anyhow::Result<(Option<String>, Option<String>)> {
        // Ollama specific pre-processing if any could go here.
        // For now, delegate entirely to OpenAI-compatible logic.
        self.inner.send(data, tool_schemas).await
    }

    async fn send_as_verifier(
        &mut self,
        data: Vec<DataSource>,
        tool_schema: Value,
    ) -> anyhow::Result<Value> {
        self.inner.send_as_verifier(data, tool_schema).await
    }
}
