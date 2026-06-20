use crate::core::session::ActiveSession;
use anyhow;
use std::collections::HashMap;

impl ActiveSession {
    pub(crate) fn execute_tool(
        &mut self,
        name: &str,
        args: &HashMap<String, serde_json::Value>,
    ) -> anyhow::Result<serde_json::Value> {
        let config = self.ctx.config_manager.get_config()?;
        // Clone the tool function out of the registry so the read-lock is not
        // held across the (potentially long) tool execution.
        let func = {
            let registry = self
                .ctx
                .tool_registry
                .read()
                .unwrap_or_else(|p| p.into_inner());
            registry.tools.get(name).map(|tool| tool.func.clone())
        };
        match func {
            Some(func) => func(args.clone(), config),
            None => Err(anyhow::anyhow!("Tool not found: {name}")),
        }
    }
}
