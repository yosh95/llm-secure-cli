use crate::core::session::ActiveSession;
use anyhow;
use std::collections::HashMap;

impl ActiveSession {
    pub(crate) async fn execute_tool(
        &mut self,
        name: &str,
        args: &HashMap<String, serde_json::Value>,
    ) -> anyhow::Result<serde_json::Value> {
        let config = self.ctx.config_manager.get_config()?;
        let registry = self.ctx.tool_registry.read().await;
        if let Some(tool) = registry.tools.get(name) {
            (tool.func)(args.clone(), config).await
        } else {
            Err(anyhow::anyhow!("Tool not found: {name}"))
        }
    }
}
