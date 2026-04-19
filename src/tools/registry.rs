use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub type ToolFunc = Arc<dyn Fn(HashMap<String, Value>) -> anyhow::Result<Value> + Send + Sync>;

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
    fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

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

            let props = obj.get_mut("properties").unwrap().as_object_mut().unwrap();
            if !props.contains_key("explanation") {
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
        let name = tool["name"].as_str().unwrap().to_string();
        let original_name = tool["original_name"].as_str().unwrap().to_string();
        let server_name = tool["server_name"].as_str().unwrap().to_string();
        let description = tool["description"].as_str().unwrap_or("").to_string();
        let parameters = tool["parameters"].clone();

        let func_server_name = server_name.clone();
        let func_original_name = original_name.clone();

        let func: ToolFunc = Arc::new(move |args| {
            let mcp = &crate::tools::mcp::manager::MCP_MANAGER;
            let rt = tokio::runtime::Handle::current();
            let result = tokio::task::block_in_place(|| {
                rt.block_on(mcp.call_tool(&func_server_name, &func_original_name, json!(args)))
            });

            match result {
                Ok(s) => Ok(json!(s)),
                Err(e) => Err(anyhow::anyhow!("MCP Error: {}", e)),
            }
        });

        self.register(&name, &description, parameters, func, false);
    }

    /// Get OpenAI-compatible tool schemas
    pub fn get_tool_schemas(&self) -> Vec<Value> {
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

    /// Get Gemini-compatible function declarations
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

    /// Get Anthropic-compatible tool schemas
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

pub static REGISTRY: Lazy<Mutex<ToolRegistry>> = Lazy::new(|| {
    let mut r = ToolRegistry::new();
    register_builtin_tools(&mut r);
    Mutex::new(r)
});

pub async fn initialize_remote_tools() -> anyhow::Result<()> {
    let mcp_manager = &crate::tools::mcp::manager::MCP_MANAGER;
    let tools = mcp_manager.initialize_servers().await?;

    let config = crate::config::CONFIG_MANAGER.get_config();
    let allowed_tools = config.security.allowed_tools;

    let mut registry = REGISTRY.lock().unwrap();
    for tool in tools {
        if let Some(ref allowed) = allowed_tools {
            let name = tool["name"].as_str().unwrap_or("");
            if !allowed.contains(&name.to_string()) {
                continue;
            }
        }
        registry.register_remote_tool(&tool);
    }

    Ok(())
}

fn register_builtin_tools(r: &mut ToolRegistry) {
    let config = crate::config::CONFIG_MANAGER.get_config();
    let allowed_tools = config.security.allowed_tools;

    let maybe_register = |r: &mut ToolRegistry,
                              name: &str,
                              description: &str,
                              parameters: Value,
                              func: ToolFunc| {
        if let Some(ref allowed) = allowed_tools {
            if !allowed.contains(&name.to_string()) {
                return;
            }
        }
        r.register(name, description, parameters, func, true);
    };

    maybe_register(
        r,
        "list_files_in_directory",
        "List files in a directory.",
        json!({
            "type": "object",
            "properties": {
                "directory": {
                    "type": "string",
                    "description": "Target directory (default: current directory)."
                },
                "depth": {
                    "type": "integer",
                    "description": "Maximum depth for recursive listing.",
                    "default": 1
                },
                "ignore_patterns": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "List of patterns to ignore (e.g. ['node_modules'])."
                },
                "include_hidden": {
                    "type": "boolean",
                    "description": "If true, show hidden files and directories.",
                    "default": false
                },
                "max_files": {
                    "type": "integer",
                    "description": "Maximum number of files to list. (Default: 500)",
                    "default": 500
                }
            }
        }),
        Arc::new(crate::tools::builtin::file_ops::list_files_in_directory),
    );

    maybe_register(
        r,
        "read_file_content",
        "Read content from a text file or PDF. For PDFs, text content will be extracted. \
         IMPORTANT: This tool can read up to 500 lines or 30000 characters at once. \
         If a file is longer, the tail will be omitted. \
         Use 'start_line' and 'end_line' to read specific chunks of large files.",
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path."},
                "start_line": {
                    "type": "integer",
                    "description": "First line to read (1-indexed).",
                    "default": 1
                },
                "end_line": {
                    "type": "integer",
                    "description": "Last line to read (Max 500 lines from start_line recommended)."
                },
                "with_line_numbers": {
                    "type": "boolean",
                    "description": "If true, adds line numbers to the output.",
                    "default": false
                }
            },
            "required": ["path"]
        }),
        Arc::new(crate::tools::builtin::file_ops::read_file_content),
    );

    maybe_register(
        r,
        "grep_files",
        "Search for a regex pattern in files within a directory (like grep). \
         Automatically excludes common junk directories like .git, node_modules, and \
         cache to provide clean and fast results.",
        json!({
            "type": "object",
            "properties": {
                "directory": {
                    "type": "string",
                    "description": "Directory to search in (default: current directory)."
                },
                "query": {
                    "type": "string",
                    "description": "Regex pattern to search for in file contents."
                },
                "file_pattern": {
                    "type": "string",
                    "description": "File pattern to include (e.g., '*.py')."
                }
            },
            "required": ["query"]
        }),
        Arc::new(crate::tools::builtin::file_ops::grep_files),
    );

    maybe_register(
        r,
        "search_files",
        "Search for files or directories by name pattern within a directory.",
        json!({
            "type": "object",
            "properties": {
                "directory": {
                    "type": "string",
                    "description": "Directory to search in (default: current directory)."
                },
                "pattern": {
                    "type": "string",
                    "description": "Pattern to match (e.g., 'test*.py' or '*config*')."
                },
                "exclude_patterns": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "List of patterns to exclude."
                }
            },
            "required": ["pattern"]
        }),
        Arc::new(crate::tools::builtin::file_ops::search_files),
    );

    maybe_register(
        r,
        "edit_file",
        "Edit a file by replacing a specific block of text. \
         The search string must match the file content exactly.",
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Path to the file to edit."},
                "search": {
                    "type": "string",
                    "description": "The block of text to find in the file."
                },
                "replace": {
                    "type": "string",
                    "description": "The new text to replace the found block with."
                },
                "dry_run": {
                    "type": "boolean",
                    "description": "If true, show diff without applying changes.",
                    "default": false
                }
            },
            "required": ["path", "search", "replace"]
        }),
        Arc::new(crate::tools::builtin::file_modification::edit_file),
    );

    maybe_register(
        r,
        "create_or_overwrite_file",
        "Write full content to a file. Overwrites existing files.",
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Path to save the file."},
                "content": {
                    "type": "string",
                    "description": "The complete file content to write. This field is REQUIRED and must contain the full text of the file. Do not omit this field."
                }
            },
            "required": ["path", "content"]
        }),
        Arc::new(crate::tools::builtin::file_modification::create_or_overwrite_file),
    );

    maybe_register(
        r,
        "read_url_content",
        "Fetch a web page URL or PDF URL and convert the content to Markdown or text. \
         For PDFs, text content will be extracted. \
         IMPORTANT: This tool can read up to 500 lines or 30000 characters at once. \
         If the content is longer, the tail will be omitted. \
         Use 'start_line' and 'end_line' to read specific chunks of large pages.",
        json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "Target URL."},
                "start_line": {
                    "type": "integer",
                    "description": "First line to read (1-indexed).",
                    "default": 1
                },
                "end_line": {
                    "type": "integer",
                    "description": "Last line to read (Max 500 lines from start_line recommended)."
                }
            },
            "required": ["url"]
        }),
        Arc::new(crate::tools::builtin::web::read_url_content),
    );

    // Brave Search: register only if API key is available
    let brave_key = crate::config::CONFIG_MANAGER.get_api_key("brave");
    if brave_key.is_some() {
        maybe_register(
            r,
            "brave_search",
            "Search the web for current information using the Brave Search API. \
             Returns a list of relevant search results including titles, snippets, and URLs. \
             Use this tool when you need to find information from the internet.",
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query to execute."
                    },
                    "count": {
                        "type": "integer",
                        "description": "Number of results to return (max 20).",
                        "default": 10
                    }
                },
                "required": ["query"]
            }),
            Arc::new(crate::tools::builtin::web::brave_search),
        );
    }

    maybe_register(
        r,
        "execute_command",
        "Execute a system command directly without a shell. \
         Use 'command' for the executable (e.g., 'ls', 'git') and 'args' for its arguments. \
         This is secure against shell injection and handles special characters like backticks or quotes correctly.",
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The executable to run."
                },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "List of arguments to pass to the executable."
                }
            },
            "required": ["command", "args"]
        }),
        Arc::new(crate::tools::builtin::shell::execute_command),
    );
}
