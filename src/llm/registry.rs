use crate::llm::base::LlmClient;
use std::collections::HashMap;
use std::sync::Arc;

pub type ClientFactory = Arc<
    dyn Fn(&str, bool, bool, &crate::config::ConfigManager) -> Box<dyn LlmClient> + Send + Sync,
>;

pub struct ClientRegistry {
    factories: HashMap<String, ClientFactory>,
}

impl Default for ClientRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientRegistry {
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
        }
    }

    pub fn register(&mut self, name: &str, factory: ClientFactory) {
        self.factories.insert(name.to_string(), factory);
    }

    pub fn create_client(
        &self,
        name: &str,
        model: &str,
        stdout: bool,
        raw: bool,
        config_manager: &crate::config::ConfigManager,
    ) -> Option<Box<dyn LlmClient>> {
        self.factories
            .get(name)
            .map(|f| f(model, stdout, raw, config_manager))
    }

    pub fn list_aliases(&self) -> Vec<String> {
        self.factories.keys().cloned().collect()
    }
}
