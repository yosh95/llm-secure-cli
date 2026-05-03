use crate::cli::ui;
use crate::tools::mcp::client::{ClientSession, StdioServerParameters};
use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct McpManager {
    sessions: Arc<Mutex<HashMap<String, ClientSession>>>,
    cached_tools: Arc<Mutex<Vec<Value>>>,
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            cached_tools: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl Default for McpManager {
    fn default() -> Self {
        Self::new()
    }
}

impl McpManager {
    pub async fn initialize_servers(
        &self,
        config_manager: &crate::config::ConfigManager,
    ) -> Result<Vec<Value>> {
        {
            let cached_tools = self.cached_tools.lock().await;
            if !cached_tools.is_empty() {
                return Ok(cached_tools.clone());
            }
        }

        let config = config_manager.get_config()?;
        let mut all_tools = Vec::new();
        let mut join_set = tokio::task::JoinSet::new();

        for server_cfg in config.mcp_servers.clone() {
            {
                let sessions = self.sessions.lock().await;
                if sessions.contains_key(&server_cfg.name) {
                    continue;
                }
            }

            join_set.spawn(async move {
                ui::report_success(&format!(
                    "Connecting to MCP server '{}'...",
                    server_cfg.name
                ));

                let params = StdioServerParameters {
                    command: server_cfg.command.clone(),
                    args: server_cfg.args.clone(),
                    env: None,
                };

                match ClientSession::start(params).await {
                    Ok(session) => {
                        if let Err(e) = session.initialize().await {
                            return Err(anyhow!(
                                "Failed to initialize '{}': {}",
                                server_cfg.name,
                                e
                            ));
                        }

                        match session.list_tools().await {
                            Ok(result) => {
                                let mut namespaced_tools = Vec::new();
                                for tool in result.tools {
                                    let namespaced_name =
                                        format!("{}__{}", server_cfg.name, tool.name);
                                    namespaced_tools.push(json!({
                                        "name": namespaced_name,
                                        "original_name": tool.name,
                                        "server_name": server_cfg.name,
                                        "description": tool.description,
                                        "parameters": tool.input_schema,
                                    }));
                                }
                                Ok((server_cfg.name, session, namespaced_tools))
                            }
                            Err(e) => Err(anyhow!(
                                "Failed to list tools for '{}': {}",
                                server_cfg.name,
                                e
                            )),
                        }
                    }
                    Err(e) => Err(anyhow!("Failed to connect to '{}': {}", server_cfg.name, e)),
                }
            });
        }

        while let Some(res) = join_set.join_next().await {
            match res {
                Ok(Ok((name, session, tools))) => {
                    ui::report_success(&format!(
                        "[OK] Connected to MCP server '{}' ({} tools).",
                        name,
                        tools.len()
                    ));
                    all_tools.extend(tools);
                    self.sessions.lock().await.insert(name, session);
                }
                Ok(Err(e)) => ui::report_error(&e.to_string()),
                Err(e) => ui::report_error(&format!("Task panicked: {}", e)),
            }
        }

        let mut cached = self.cached_tools.lock().await;
        *cached = all_tools.clone();
        Ok(all_tools)
    }

    pub async fn call_tool(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: Value,
    ) -> Result<String> {
        let sessions = self.sessions.lock().await;
        let session = sessions
            .get(server_name)
            .ok_or_else(|| anyhow!("MCP server '{}' not connected.", server_name))?;

        // Simplified argument distribution (similar to Python version)
        let mut tool_args = json!({});
        let mut metadata = json!({});

        if let Some(args_obj) = arguments.as_object() {
            for (key, value) in args_obj {
                if key == "explanation" || key.starts_with("__") {
                    metadata[key] = value.clone();
                } else {
                    tool_args[key] = value.clone();
                }
            }
        }

        let result = session
            .call_tool(tool_name, tool_args, None, Some(metadata))
            .await?;

        let mut output = Vec::new();
        for content in result.content {
            if content.content_type == "text" {
                if let Some(text) = content.text {
                    output.push(text);
                }
            } else {
                output.push(format!("[Binary/Other content: {}]", content.content_type));
            }
        }

        if output.is_empty() {
            Ok("No output from tool.".to_string())
        } else {
            Ok(output.join("\n"))
        }
    }
}
