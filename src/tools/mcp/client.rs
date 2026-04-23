use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StdioServerParameters {
    pub command: String,
    pub args: Vec<String>,
    pub env: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Content {
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(rename = "mimeType", skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub content: Vec<Content>,
    #[serde(rename = "isError", default)]
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDescription {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListToolsResult {
    pub tools: Vec<ToolDescription>,
}

pub struct JSONRPCProtocol {
    msg_id: i64,
}

impl JSONRPCProtocol {
    pub fn new() -> Self {
        Self { msg_id: 0 }
    }
}

impl Default for JSONRPCProtocol {
    fn default() -> Self {
        Self::new()
    }
}

impl JSONRPCProtocol {
    pub fn next_id(&mut self) -> i64 {
        self.msg_id += 1;
        self.msg_id
    }

    pub fn create_request(&mut self, method: &str, params: Option<Value>) -> (i64, Value) {
        let id = self.next_id();
        let req = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params.unwrap_or(json!({})),
            "id": id,
        });
        (id, req)
    }
}

pub struct ClientSession {
    #[allow(dead_code)]
    child: Child,
    request_tx: mpsc::UnboundedSender<(Value, oneshot::Sender<Result<Value>>)>,
}

impl ClientSession {
    pub async fn start(params: StdioServerParameters) -> Result<Self> {
        let mut cmd = Command::new(&params.command);
        cmd.args(&params.args);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::inherit());

        if let Some(env) = params.env {
            cmd.envs(env);
        }

        let mut child = cmd.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("Failed to open stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("Failed to open stdout"))?;

        let (request_tx, mut request_rx) =
            mpsc::unbounded_channel::<(Value, oneshot::Sender<Result<Value>>)>();

        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            let mut writer = stdin;
            let mut pending_requests: HashMap<i64, oneshot::Sender<Result<Value>>> = HashMap::new();

            loop {
                tokio::select! {
                    Some((req, response_tx)) = request_rx.recv() => {
                        let id = req.get("id").and_then(|id| id.as_i64()).unwrap_or(0);
                        pending_requests.insert(id, response_tx);
                        let mut line = serde_json::to_string(&req).unwrap();
                        line.push('\n');
                        if let Err(e) = writer.write_all(line.as_bytes()).await {
                            log::error!("Failed to write to MCP server: {}", e);
                            break;
                        }
                    }
                    line_result = reader.next_line() => {
                        match line_result {
                            Ok(Some(line)) => {
                                if !line.trim().starts_with('{') {
                                    continue;
                                }
                                if let Ok(message) = serde_json::from_str::<Value>(&line) {
                                    if let Some(id_val) = message.get("id") {
                                        if let Some(id) = id_val.as_i64() {
                                            if let Some(tx) = pending_requests.remove(&id) {
                                                if let Some(error) = message.get("error") {
                                                    let msg = error.get("message").and_then(|m| m.as_str()).unwrap_or("Unknown error");
                                                    let _ = tx.send(Err(anyhow!("{}", msg)));
                                                } else {
                                                    let result = message.get("result").cloned().unwrap_or(Value::Null);
                                                    let _ = tx.send(Ok(result));
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            Ok(None) => break,
                            Err(e) => {
                                log::error!("Error reading from MCP server: {}", e);
                                break;
                            }
                        }
                    }
                }
            }
        });

        Ok(Self { child, request_tx })
    }

    async fn send_request(&self, method: &str, params: Option<Value>) -> Result<Value> {
        // We need a local counter for IDs because we are sending the whole Value.
        // In the Python version, the protocol is owned by the session.
        // For simplicity, I'll generate a unique ID here.
        static NEXT_ID: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1);
        let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let req = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params.unwrap_or(json!({})),
            "id": id,
        });

        let (tx, rx) = oneshot::channel();
        self.request_tx
            .send((req, tx))
            .map_err(|_| anyhow!("Failed to send request to loop"))?;
        rx.await?
    }

    pub async fn initialize(&self) -> Result<Value> {
        self.send_request(
            "initialize",
            Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "llm-secure-cli-client", "version": "0.1.0"},
            })),
        )
        .await
    }

    pub async fn list_tools(&self) -> Result<ListToolsResult> {
        let response = self.send_request("tools/list", None).await?;
        let tools: ListToolsResult = serde_json::from_value(response)?;
        Ok(tools)
    }

    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Value,
        trace_id: Option<String>,
        metadata: Option<Value>,
    ) -> Result<ToolResult> {
        let mut args_payload = arguments;
        let mut meta = metadata.unwrap_or(json!({}));
        if let Some(tid) = trace_id {
            meta["trace_id"] = json!(tid);
        }

        if let Some(obj) = args_payload.as_object_mut() {
            obj.insert("_meta".to_string(), meta);
        }

        let response = self
            .send_request(
                "tools/call",
                Some(json!({
                    "name": name,
                    "arguments": args_payload,
                })),
            )
            .await?;

        let result: ToolResult = serde_json::from_value(response)?;
        Ok(result)
    }
}

pub struct FastMcp {
    pub name: String,
    pub tools: HashMap<String, Box<dyn Fn(Value) -> Result<Value> + Send + Sync>>,
}

impl FastMcp {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            tools: HashMap::new(),
        }
    }

    pub fn tool<F>(&mut self, name: &str, func: F)
    where
        F: Fn(Value) -> Result<Value> + Send + Sync + 'static,
    {
        self.tools.insert(name.to_string(), Box::new(func));
    }

    pub async fn run(&self) -> Result<()> {
        let stdin = tokio::io::stdin();
        let mut stdout = tokio::io::stdout();
        let mut reader = BufReader::new(stdin).lines();

        while let Ok(Some(line)) = reader.next_line().await {
            if !line.trim().starts_with('{') {
                continue;
            }

            if let Ok(message) = serde_json::from_str::<Value>(&line) {
                let id = message.get("id").cloned();
                let method = message.get("method").and_then(|m| m.as_str());

                match method {
                    Some("initialize") => {
                        let resp = json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "protocolVersion": "2024-11-05",
                                "capabilities": {"tools": {"listChanged": false}},
                                "serverInfo": {"name": self.name, "version": "0.1.0"},
                            }
                        });
                        let mut resp_line = serde_json::to_string(&resp)?;
                        resp_line.push('\n');
                        stdout.write_all(resp_line.as_bytes()).await?;
                        stdout.flush().await?;
                    }
                    Some("tools/list") => {
                        let tools_list: Vec<Value> = self
                            .tools
                            .keys()
                            .map(|name| {
                                json!({
                                    "name": name,
                                    "description": "",
                                    "inputSchema": {"type": "object", "properties": {}}
                                })
                            })
                            .collect();

                        let resp = json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "tools": tools_list
                            }
                        });
                        let mut resp_line = serde_json::to_string(&resp)?;
                        resp_line.push('\n');
                        stdout.write_all(resp_line.as_bytes()).await?;
                        stdout.flush().await?;
                    }
                    Some("tools/call") => {
                        let params = message.get("params");
                        let tool_name = params.and_then(|p| p.get("name")).and_then(|n| n.as_str());
                        let arguments = params
                            .and_then(|p| p.get("arguments"))
                            .cloned()
                            .unwrap_or(json!({}));

                        let result = if let Some(name) = tool_name {
                            if let Some(tool) = self.tools.get(name) {
                                match tool(arguments) {
                                    Ok(res) => Ok(res),
                                    Err(e) => Err(e.to_string()),
                                }
                            } else {
                                Err(format!("Tool not found: {}", name))
                            }
                        } else {
                            Err("Missing tool name".to_string())
                        };

                        let resp = match result {
                            Ok(res) => json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": {
                                    "content": [{"type": "text", "text": res.to_string()}],
                                    "isError": false
                                }
                            }),
                            Err(e) => json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "error": {
                                    "code": -32000,
                                    "message": e
                                }
                            }),
                        };
                        let mut resp_line = serde_json::to_string(&resp)?;
                        resp_line.push('\n');
                        stdout.write_all(resp_line.as_bytes()).await?;
                        stdout.flush().await?;
                    }
                    _ => {
                        if let Some(id_val) = id {
                            let resp = json!({
                                "jsonrpc": "2.0",
                                "id": id_val,
                                "error": {
                                    "code": -32601,
                                    "message": "Method not found"
                                }
                            });
                            let mut resp_line = serde_json::to_string(&resp)?;
                            resp_line.push('\n');
                            stdout.write_all(resp_line.as_bytes()).await?;
                            stdout.flush().await?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
