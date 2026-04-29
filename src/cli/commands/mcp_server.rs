use crate::cli::ui;
use crate::tools::mcp::client::FastMcp;
use anyhow::Result;
use std::collections::HashMap;

use crate::core::context::AppContext;
use std::sync::Arc;

pub async fn run_mcp_server(ctx: Arc<AppContext>) -> Result<()> {
    let config = ctx.config_manager.get_config();
    ui::report_success("Starting LLM-SECURE-CLI MCP Server (stdio mode)...");

    let mut mcp = FastMcp::new("llm-secure-cli-server");

    // Register all tools from the registry
    {
        let registry = ctx.tool_registry.lock().unwrap();
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
                        crate::security::audit::log_audit(crate::security::audit::AuditParams {
                            event_type: "mcp_blocked",
                            tool_name: &tool_name_for_closure,
                            args: args.clone(),
                            output: None,
                            exit_code: Some(1),
                            error: Some(&e),
                            context: None,
                            config: &config,
                        });
                        return Err(anyhow::anyhow!(e));
                    }

                    // Audit: Log the execution
                    crate::security::audit::log_audit(crate::security::audit::AuditParams {
                        event_type: "mcp_execution",
                        tool_name: &tool_name_for_closure,
                        args: args.clone(),
                        output: None,
                        exit_code: Some(0),
                        error: None,
                        context: None,
                        config: &config,
                    });

                    (tool_func)(args_map, config).await
                }
            });
        }
    }

    mcp.run().await
}
