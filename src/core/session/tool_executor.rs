use crate::core::session::ChatSession;
use anyhow;
use std::collections::HashMap;

impl ChatSession {
    pub(crate) async fn execute_tool(
        &mut self,
        name: &str,
        args: HashMap<String, serde_json::Value>,
    ) -> anyhow::Result<serde_json::Value> {
        // Remote MCP tools: execute asynchronously via MCP manager directly
        // to avoid tokio::task::block_in_place in the registry closure.
        if name.contains("__") {
            let parts: Vec<&str> = name.splitn(2, "__").collect();
            if parts.len() == 2 {
                let server_name = parts[0];
                let tool_name = parts[1];
                let mcp = &crate::tools::mcp::manager::MCP_MANAGER;
                match mcp
                    .call_tool(server_name, tool_name, serde_json::json!(args))
                    .await
                {
                    Ok(result) => return Ok(serde_json::json!(result)),
                    Err(e) => return Err(anyhow::anyhow!("MCP Error: {}", e)),
                }
            }
        }

        // Local built-in tools: asynchronous execution
        let fut = {
            let config = crate::config::CONFIG_MANAGER.get_config();
            let registry = crate::tools::registry::REGISTRY.lock().unwrap();
            registry
                .tools
                .get(name)
                .map(|tool| (tool.func)(args, config))
        };

        if let Some(fut) = fut {
            fut.await
        } else {
            Err(anyhow::anyhow!("Tool not found: {}", name))
        }
    }
}
