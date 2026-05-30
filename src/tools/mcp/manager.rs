use crate::cli::ui;
use crate::tools::mcp::client::ClientSession;
use anyhow::{Result, anyhow};
use rmcp::model::RawContent;
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

            let server_name = server_cfg.name.clone();
            let transport = server_cfg.transport.clone();
            let command = server_cfg.command.clone();
            let args = server_cfg.args.clone();
            let api_url = server_cfg.api_url.clone();

            join_set.spawn(async move {
                ui::report_success(&format!("Connecting to MCP server '{}'...", server_name));

                // Determine transport type and connect
                let session_result = match transport.as_str() {
                    "streamable-http" | "http" | "https" => {
                        let url = api_url.ok_or_else(|| {
                            anyhow!("api_url is required for streamable-http transport")
                        })?;
                        ClientSession::start_http(&url).await
                    }
                    _ => {
                        // Default: stdio transport
                        ClientSession::start_stdio(
                            crate::tools::mcp::client::StdioServerParameters {
                                command,
                                args,
                                env: None,
                            },
                        )
                        .await
                    }
                };

                match session_result {
                    Ok(mut session) => match session.list_tools().await {
                        Ok(tools) => {
                            let mut namespaced_tools = Vec::new();
                            for tool in tools {
                                let namespaced_name = format!("{}__{}", server_name, tool.name);
                                namespaced_tools.push(json!({
                                    "name": namespaced_name,
                                    "original_name": tool.name,
                                    "server_name": server_name,
                                    "description": tool.description,
                                    "parameters": tool.input_schema,
                                }));
                            }
                            Ok((server_name, session, namespaced_tools))
                        }
                        Err(e) => Err(anyhow!("Failed to list tools for '{}': {}", server_name, e)),
                    },
                    Err(e) => Err(anyhow!("Failed to connect to '{}': {}", server_name, e)),
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

        // Filter out internal arguments (explanation, __meta)
        let mut tool_args = json!({});
        if let Some(args_obj) = arguments.as_object() {
            for (key, value) in args_obj {
                if key != "explanation" && !key.starts_with("__") {
                    tool_args[key] = value.clone();
                }
            }
        }

        let result = session.call_tool(tool_name, tool_args).await?;

        let mut output = Vec::new();
        for content in result.content {
            match content.raw {
                RawContent::Text(ref text) => {
                    output.push(text.text.clone());
                }
                RawContent::Image(ref img) => {
                    output.push(format!("[Image: {}]", img.mime_type));
                }
                RawContent::Resource(ref _res) => {
                    output.push("[Resource]".to_string());
                }
                RawContent::Audio(ref audio) => {
                    output.push(format!("[Audio: {}]", audio.mime_type));
                }
                _ => {
                    output.push("[Other content type]".to_string());
                }
            }
        }

        if output.is_empty() {
            Ok("No output from tool.".to_string())
        } else {
            Ok(output.join("\n"))
        }
    }
}
