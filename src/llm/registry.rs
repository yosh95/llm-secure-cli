use crate::llm::base::LlmClient;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::Mutex;

pub type ClientFactory = fn(model: &str, stdout: bool, raw: bool) -> Box<dyn LlmClient>;

pub struct ClientRegistry {
    factories: HashMap<String, ClientFactory>,
}

pub static CLIENT_REGISTRY: Lazy<Mutex<ClientRegistry>> =
    Lazy::new(|| Mutex::new(ClientRegistry::new()));

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
    ) -> Option<Box<dyn LlmClient>> {
        self.factories.get(name).map(|f| f(model, stdout, raw))
    }

    pub fn list_aliases(&self) -> Vec<String> {
        self.factories.keys().cloned().collect()
    }
}
