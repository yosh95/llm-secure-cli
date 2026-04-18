use crate::cli::ui;
use crate::tools::mcp::client::FastMcp;
use anyhow::Result;
use std::collections::HashMap;

pub async fn run_mcp_server() -> Result<()> {
    ui::report_success("Starting LLM-SECURE-CLI MCP Server (stdio mode)...");

    let mut mcp = FastMcp::new("llm-secure-cli-server");

    // Register all tools from the registry
    {
        let registry = crate::tools::registry::REGISTRY.lock().unwrap();
        for (name, tool) in &registry.tools {
            if !tool.is_local {
                continue; // Don't re-export remote tools
            }

            let tool_name = name.clone();
            let tool_func = tool.func.clone();

            mcp.tool(&tool_name, move |args| {
                // Convert Value to HashMap<String, Value>
                let mut args_map = HashMap::new();
                if let Some(obj) = args.as_object() {
                    for (k, v) in obj {
                        args_map.insert(k.clone(), v.clone());
                    }
                }

                // Security: In a real implementation, we would apply secure_tool_wrapper here
                // similar to the Python version.

                (tool_func)(args_map)
            });
        }
    }

    mcp.run().await
}
