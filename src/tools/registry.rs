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

    let config = config_manager.get_config()?;
    let allowed_tools = config.security.allowed_tools.as_ref();

    let mut registry = registry.write().await;
    for tool in tools {
        if let Some(allowed) = allowed_tools {
            let name = tool["name"].as_str().unwrap_or_default();
            if allowed.is_empty() {
                continue; // Empty list disables all tools
            }
            if !allowed.iter().any(|pattern| {
                if pattern == "*" {
                    true
                } else if pattern.ends_with('*') {
                    name.starts_with(pattern.trim_end_matches('*'))
                } else {
                    pattern == name
                }
            }) {
                continue;
            }
        }
        registry.register_remote_tool(&tool);
    }

    Ok(())
}

pub fn register_builtin_tools(r: &mut ToolRegistry, config_manager: &crate::config::ConfigManager) {
    let allowed_tools = if let Ok(config) = config_manager.get_config() {
        config.security.allowed_tools.clone()
    } else {
        None
    };

    let maybe_register =
        |r: &mut ToolRegistry, name: &str, description: &str, parameters: Value, func: ToolFunc| {
            if let Some(ref allowed) = allowed_tools {
                if allowed.is_empty() {
                    return; // Empty list disables all tools
                }
                if !allowed.iter().any(|pattern| {
                    if pattern == "*" {
                        true
                    } else if pattern.ends_with('*') {
                        name.starts_with(pattern.trim_end_matches('*'))
                    } else {
                        pattern == name
                    }
                }) {
                    return;
                }
            }
            r.register(name, description, parameters, func, true);
        };

    maybe_register(
        r,
        "list_files",
        "List files and directories in a directory.",
        json!({
            "type": "object",
            "properties": {
                "directory": { "type": "string", "description": "Target directory (default: current directory)." },
                "depth": { "type": "integer", "description": "Maximum depth for recursive listing.", "default": 1 },
                "ignore_patterns": { "type": "array", "items": {"type": "string"}, "description": "List of patterns to ignore." },
                "include_hidden": { "type": "boolean", "description": "If true, show hidden files.", "default": false },
                "max_files": { "type": "integer", "description": "Max files to list.", "default": 500 }
            }
        }),
        Arc::new(|args, config| {
            Box::pin(async move { crate::tools::builtin::file_ops::list_files(args, config) })
        }),
    );

    maybe_register(
        r,
        "read_file",
        "Read content from a local text file or PDF.",
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Local file path (not a URL)."},
                "start_line": {"type": "integer", "default": 1},
                "end_line": {"type": "integer"},
                "with_line_numbers": {"type": "boolean", "default": false}
            },
            "required": ["path"]
        }),
        Arc::new(|args, config| {
            Box::pin(async move { crate::tools::builtin::file_ops::read_file(args, config) })
        }),
    );

    maybe_register(
        r,
        "grep_files",
        "Search for a regex pattern in files.",
        json!({
            "type": "object",
            "properties": {
                "directory": {"type": "string"},
                "query": {"type": "string"},
                "file_pattern": {"type": "string"}
            },
            "required": ["query"]
        }),
        Arc::new(|args, config| {
            Box::pin(async move { crate::tools::builtin::file_ops::grep_files(args, config) })
        }),
    );

    maybe_register(
        r,
        "search_files",
        "Search for files by name pattern.",
        json!({
            "type": "object",
            "properties": {
                "directory": {"type": "string"},
                "pattern": {"type": "string"},
                "exclude_patterns": {"type": "array", "items": {"type": "string"}}
            },
            "required": ["pattern"]
        }),
        Arc::new(|args, config| {
            Box::pin(async move { crate::tools::builtin::file_ops::search_files(args, config) })
        }),
    );

    maybe_register(
        r,
        "edit_file",
        "Edit a file by replacing a block of text. \
         IMPORTANT: provide the exact block of text to be replaced (old) and the exact block to replace it with (new). \
         Do NOT use ellipsis (...) or omit any lines in the search or replace blocks; they must match the file content perfectly. \
         Do NOT escape newlines; provide raw newline characters in your tool call. \
         This tool expects EXACTLY one match in the file to succeed.",
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path."},
                "old": {"type": "string", "description": "The exact block of text to search for. Use raw newlines, not '\\n'. Do NOT use ellipsis (...)."},
                "new": {"type": "string", "description": "The block of text to replace it with. Use raw newlines, not '\\n'. Do NOT use ellipsis (...)."}
            },
            "required": ["path", "old", "new"]
        }),
        Arc::new(|args, config| {
            Box::pin(
                async move { crate::tools::builtin::file_modification::edit_file(args, config) },
            )
        }),
    );

    maybe_register(
        r,
        "create_or_overwrite_file",
        "Write full content to a file. Overwrites existing files. \
         Do NOT escape newlines; provide raw newline characters in your tool call.",
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path."},
                "content": {"type": "string", "description": "The content to write. Use raw newlines, not '\\n'."}
            },
            "required": ["path", "content"]
        }),
        Arc::new(|args, config| {
            Box::pin(async move {
                crate::tools::builtin::file_modification::create_or_overwrite_file(args, config)
            })
        }),
    );

    maybe_register(
        r,
        "read_url_content",
        "Fetch a web page or PDF from a URL.",
        json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "URL to fetch (http/https)."},
                "start_line": {"type": "integer", "default": 1},
                "end_line": {"type": "integer"}
            },
            "required": ["url"]
        }),
        Arc::new(|args, config| {
            Box::pin(
                async move { crate::tools::builtin::web::read_url_content(args, config).await },
            )
        }),
    );

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
            "Execute arbitrary Python code.",
            json!({
                "type": "object",
                "properties": {
                    "code": {
                        "type": "string",
                        "description": "Python source code to execute. Can use any standard library or installed third-party package. Use print() for output."
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
