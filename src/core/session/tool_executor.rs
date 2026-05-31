use crate::core::session::ActiveSession;
use anyhow;
use std::collections::HashMap;

impl ActiveSession {
    pub(crate) async fn execute_tool(
        &mut self,
        name: &str,
        args: &HashMap<String, serde_json::Value>,
    ) -> anyhow::Result<serde_json::Value> {
        // Look up the tool in the registry to determine whether it is local or remote.
        // This avoids relying on naming conventions (e.g. "__" separator) to classify
        // tools, which could collide with legitimate tool names.
        let is_local = {
            let registry = self.ctx.tool_registry.read().await;
            registry.tools.get(name).map(|t| t.is_local)
        };

        match is_local {
            // Remote MCP tools: execute asynchronously via MCP manager directly
            // to avoid tokio::task::block_in_place in the registry closure.
            // MCP tools are namespaced with "__" separator: "server__tool"
            Some(false) => {
                if let Some((server_name, tool_name)) = name.split_once("__")
                    && !server_name.is_empty()
                    && !tool_name.is_empty()
                {
                    let mcp = &self.ctx.mcp_manager;
                    match mcp
                        .call_tool(server_name, tool_name, serde_json::json!(args))
                        .await
                    {
                        Ok(result) => return Ok(serde_json::json!(result)),
                        Err(e) => return Err(anyhow::anyhow!("MCP Error: {e}")),
                    }
                }
                Err(anyhow::anyhow!(
                    "MCP tool '{name}' has invalid namespaced format"
                ))
            }
            // Local built-in tools: execute via the registry
            Some(true) => {
                let config = self.ctx.config_manager.get_config()?;
                let registry = self.ctx.tool_registry.read().await;
                if let Some(tool) = registry.tools.get(name) {
                    (tool.func)(args.clone(), config).await
                } else {
                    Err(anyhow::anyhow!("Tool not found: {name}"))
                }
            }
            // Tool not registered at all
            None => Err(anyhow::anyhow!("Tool not found: {name}")),
        }
    }
}
