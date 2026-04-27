use crate::cli::ui;
use crate::config::models::AppConfig;
use crate::tools::mcp::client::FastMcp;
use anyhow::Result;
use std::collections::HashMap;

pub async fn run_mcp_server(config: AppConfig) -> Result<()> {
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
            let tool_name_for_closure = tool_name.clone();
            let config = config.clone();

            mcp.tool(&tool_name, move |args| {
                let tool_func = tool_func.clone();
                let tool_name_for_closure = tool_name_for_closure.clone();
                let config = config.clone();
                async move {
                    // Convert Value to HashMap<String, Value>
                    let mut args_map = HashMap::new();
                    let mut json_map = serde_json::Map::new();
                    if let Some(obj) = args.as_object() {
                        for (k, v) in obj {
                            args_map.insert(k.clone(), v.clone());
                            json_map.insert(k.clone(), v.clone());
                        }
                    }

                    // Security: Apply Phase 1 security checks
                    if let Err(e) = crate::security::validate_tool_call(
                        &tool_name_for_closure,
                        &json_map,
                        &config.security,
                    ) {
                        crate::security::audit::log_audit(
                            "mcp_blocked",
                            &tool_name_for_closure,
                            args.clone(),
                            None,
                            Some(1),
                            Some(&e),
                            None,
                        );
                        return Err(anyhow::anyhow!(e));
                    }

                    // Audit: Log the execution
                    crate::security::audit::log_audit(
                        "mcp_execution",
                        &tool_name_for_closure,
                        args.clone(),
                        None,
                        Some(0),
                        None,
                        None,
                    );

                    (tool_func)(args_map, config).await
                }
            });
        }
    }

    mcp.run().await
}
