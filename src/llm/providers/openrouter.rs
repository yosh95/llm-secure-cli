use crate::config::ConfigManager;
use crate::llm::base::LlmClient;
use crate::llm::models::{ClientState, DataSource};
use crate::llm::providers::openai_compatible::{OpenAiCompatibleClient, PayloadFormatter};
use async_trait::async_trait;
use serde_json::{Value, json};

pub struct OpenRouterFormatter;
impl PayloadFormatter for OpenRouterFormatter {
    fn format_pdf(&self, data: &str, filename: Option<&str>) -> Option<Value> {
        // OpenRouter unique "file" format
        Some(json!({
            "type": "file",
            "file": {
                "filename": filename.unwrap_or("document.pdf"),
                "file_data": format!("data:application/pdf;base64,{}", data)
            }
        }))
    }
}

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
        let inner = OpenAiCompatibleClient::builder(config_manager)
            .provider_name(provider_name)
            .api_url(api_url)
            .api_key(api_key)
            .model(model)
            .stdout(stdout)
            .raw(raw)
            .formatter(Box::new(OpenRouterFormatter))
            .build()?;
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
        true
    }

    async fn send(
        &mut self,
        data: Vec<DataSource>,
        tool_schemas: Vec<Value>,
    ) -> anyhow::Result<(Option<String>, Option<String>)> {
        // Delegate to inner; special logic for videos/audio can be added to formatter if needed
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
