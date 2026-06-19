use crate::config::models::AppConfig;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub type ToolFuture = Pin<Box<dyn Future<Output = anyhow::Result<Value>> + Send>>;
pub type ToolFunc = Arc<dyn Fn(HashMap<String, Value>, Arc<AppConfig>) -> ToolFuture + Send + Sync>;

pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub func: ToolFunc,
}

pub struct ToolRegistry {
    pub tools: HashMap<String, Tool>,
}

impl ToolRegistry {
    #[must_use]
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
    pub fn register(&mut self, name: &str, description: &str, parameters: Value, func: ToolFunc) {
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
            },
        );
    }

    #[must_use]
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

    #[must_use]
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

    #[must_use]
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

/// Checks whether a Python interpreter is available in the system PATH.
fn check_python_available() -> bool {
    let python3_check = std::process::Command::new("python3")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    if python3_check.is_ok() {
        return true;
    }

    let python_check = std::process::Command::new("python")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    python_check.is_ok()
}

pub fn register_builtin_tools(r: &mut ToolRegistry, config_manager: &crate::config::ConfigManager) {
    let maybe_register =
        |r: &mut ToolRegistry, name: &str, description: &str, parameters: Value, func: ToolFunc| {
            r.register(name, description, parameters, func);
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
            Arc::new(move |args, _config| {
                let key = brave_key.clone();
                Box::pin(async move { crate::tools::builtin::web::brave_search(args, &key).await })
            }),
        );
    }

    // Register execute_python only if python3 or python is available
    if check_python_available() {
        maybe_register(
            r,
            "execute_python",
            "Execute Python code. Runs in a fresh process per call. Provides stdout, stderr, and exit code as output.",
            json!({
                "type": "object",
                "properties": {
                    "code": {
                        "type": "string",
                        "description": "Python code to execute. Runs in a fresh Python process per call.",
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
