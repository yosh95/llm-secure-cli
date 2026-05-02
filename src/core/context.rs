use crate::config::ConfigManager;
use crate::llm::registry::ClientRegistry;
use crate::tools::mcp::manager::McpManager;
use crate::tools::registry::ToolRegistry;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct AppContext {
    pub config_manager: ConfigManager,
    pub tool_registry: Arc<Mutex<ToolRegistry>>,
    pub client_registry: Arc<Mutex<ClientRegistry>>,
    pub mcp_manager: McpManager,
}

impl AppContext {
    pub fn new() -> Self {
        Self {
            config_manager: ConfigManager::new(),
            tool_registry: Arc::new(Mutex::new(ToolRegistry::new())),
            client_registry: Arc::new(Mutex::new(ClientRegistry::new())),
            mcp_manager: McpManager::new(),
        }
    }
}

impl Default for AppContext {
    fn default() -> Self {
        Self::new()
    }
}
