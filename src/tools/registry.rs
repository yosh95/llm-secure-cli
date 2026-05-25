use crate::config::models::AppConfig;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;

pub type ToolFuture = Pin<Box<dyn Future<Output = anyhow::Result<Value>> + Send>>;
pub type ToolFunc = Arc<dyn Fn(HashMap<String, Value>, Arc<AppConfig>) -> ToolFuture + Send + Sync>;

pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub func: ToolFunc,
    pub is_local: bool,
}

pub struct ToolRegistry {
    pub tools: HashMap<String, Tool>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    pub fn register(
        &mut self,
        name: &str,
        description: &str,
        parameters: Value,
        func: ToolFunc,
        is_local: bool,
    ) {
        let mut params = parameters.clone();
        if let Some(obj) = params.as_object_mut() {
            if !obj.contains_key("properties") {
                obj.insert("properties".to_string(), json!({}));
            }

            if let Some(props) = obj.get_mut("properties").and_then(|v| v.as_object_mut())
                && !props.contains_key("explanation")
            {
                props.insert("explanation".to_string(), json!({
                        "type": "string",
                        "description": "A detailed explanation of why this tool is needed and what it will do, providing context for the user to approve the action."
                    }));
            }

            if let Some(req) = obj.get_mut("required").and_then(|v| v.as_array_mut()) {
                if !req.iter().any(|v| v == "explanation") {
                    req.push(json!("explanation"));
                }
            } else {
                obj.insert("required".to_string(), json!(["explanation"]));
            }
        }

        self.tools.insert(
            name.to_string(),
            Tool {
                name: name.to_string(),
                description: description.to_string(),
                parameters: params,
                func,
                is_local,
            },
        );
    }

    pub fn register_remote_tool(&mut self, tool: &Value) {
        let name = tool["name"]
            .as_str()
            .unwrap_or("unknown_remote_tool")
            .to_string();
        let description = tool["description"].as_str().unwrap_or_default().to_string();
        let parameters = tool["parameters"].clone();

        let name_for_error = name.clone();
        let func: ToolFunc = Arc::new(move |_args, _config| {
            let n = name_for_error.clone();
            Box::pin(async move {
                Err(anyhow::anyhow!(
                    "MCP tool '{}' should be executed via async path in ChatSession::execute_tool",
                    n
                ))
            })
        });

        self.register(&name, &description, parameters, func, false);
    }

    pub fn get_tool_schemas(&self) -> Vec<Value> {
        let mut tools: Vec<_> = self.tools.values().collect();
        tools.sort_by(|a, b| a.name.cmp(&b.name));

        tools
            .into_iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                })
            })
            .collect()
    }

    pub fn get_tool_schemas_gemini(&self) -> Vec<Value> {
        self.tools
            .values()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                })
            })
            .collect()
    }

    pub fn get_tool_schemas_anthropic(&self) -> Vec<Value> {
        self.tools
            .values()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters,
                })
            })
            .collect()
    }
}

pub async fn initialize_remote_tools(
    registry: Arc<RwLock<ToolRegistry>>,
    config_manager: &crate::config::ConfigManager,
    mcp_manager: &crate::tools::mcp::manager::McpManager,
) -> anyhow::Result<()> {
    let tools = mcp_manager.initialize_servers(config_manager).await?;

    let mut registry = registry.write().await;
    for tool in tools {
        registry.register_remote_tool(&tool);
    }

    Ok(())
}

pub fn register_builtin_tools(r: &mut ToolRegistry, config_manager: &crate::config::ConfigManager) {
    let maybe_register =
        |r: &mut ToolRegistry, name: &str, description: &str, parameters: Value, func: ToolFunc| {
            r.register(name, description, parameters, func, true);
        };

    if let Some(brave_key) = config_manager.get_api_key("brave") {
        maybe_register(
            r,
            "brave_search",
            "Search the web using Brave Search API.",
            json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "The search query (1-400 chars, max 50 words)."}
                },
                "required": ["query"]
            }),
            Arc::new(move |args, config| {
                let key = brave_key.clone();
                Box::pin(async move {
                    crate::tools::builtin::web::brave_search(args, config, &key).await
                })
            }),
        );
    }

    if crate::tools::builtin::python::is_python_available() {
        maybe_register(
            r,
            "execute_python",
            "Execute arbitrary Python code. Multi-purpose tool for: file ops (read/write/edit/list), grep/search (re, fnmatch), web fetching (requests/urllib), PDF text extraction (pdftotext via subprocess), data processing, AND arbitrary shell commands (via subprocess). NOT for web research—use brave_search.",
            json!({
                "type": "object",
                "properties": {
                    "code": {
                        "type": "string",
                        "description": "Python source code to execute. Can use any standard library or installed third-party package. Use print() for output. For multi-line strings, use triple quotes (\"\"\") instead of embedding literal newlines inside single-line string literals."
                    }
                },
                "required": ["code"]
            }),
            Arc::new(|args, config| {
                Box::pin(async move {
                    crate::tools::builtin::python::execute_python(args, config).await
                })
            }),
        );
    }
}
