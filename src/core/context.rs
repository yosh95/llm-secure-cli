use crate::cli::ui::UserInterface;
use crate::config::ConfigManager;
use crate::llm::registry::ClientRegistry;
use crate::tools::registry::ToolRegistry;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

pub struct AppContext {
    pub config_manager: ConfigManager,
    pub tool_registry: Arc<RwLock<ToolRegistry>>,
    pub client_registry: Arc<Mutex<ClientRegistry>>,
    pub ui: Arc<dyn UserInterface>,
}

impl AppContext {
    pub fn new(ui: Arc<dyn UserInterface>) -> Self {
        Self {
            config_manager: ConfigManager::new(),
            tool_registry: Arc::new(RwLock::new(ToolRegistry::new())),
            client_registry: Arc::new(Mutex::new(ClientRegistry::new())),
            ui,
        }
    }
}

impl Default for AppContext {
    fn default() -> Self {
        Self::new(Arc::new(crate::cli::ui::CliUi))
    }
}
