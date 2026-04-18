use crate::clients::config::CONFIG_MANAGER;
use crate::ui;
use anyhow::Result;
use rust_mcp_sdk::mcp_client::ClientRuntime;
use rust_mcp_sdk::schema::ServerMessage;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct McpManager {
    #[allow(dead_code)]
    runtimes: Arc<Mutex<HashMap<String, ClientRuntime>>>,
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            runtimes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn initialize_servers(&self) -> Result<Vec<Value>> {
        let config = CONFIG_MANAGER.get_config();
        let all_tools = Vec::new();

        for server_cfg in &config.mcp_servers {
            ui::report_success(&format!("Connecting to MCP server '{}'...", server_cfg.name));

            // Use create_with_server_launch to start the MCP server process
            let _transport = rust_mcp_sdk::StdioTransport::<ServerMessage>::create_with_server_launch(
                &server_cfg.command,
                server_cfg.args.clone(),
                None, // env
                rust_mcp_sdk::TransportOptions::default(),
            );
            
            // TODO: Initialize ClientRuntime and register handlers
        }

        Ok(all_tools)
    }
}
