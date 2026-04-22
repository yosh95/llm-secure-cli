use crate::core::session::ChatSession;
use anyhow;
use std::collections::HashMap;

impl ChatSession {
    pub(crate) fn execute_tool(
        &mut self,
        name: &str,
        args: HashMap<String, serde_json::Value>,
    ) -> anyhow::Result<serde_json::Value> {
        let registry = crate::tools::registry::REGISTRY.lock().unwrap();
        if let Some(tool) = registry.tools.get(name) {
            (tool.func)(args)
        } else {
            Err(anyhow::anyhow!("Tool not found: {}", name))
        }
    }
}
