use crate::security::pqc::PQCVariant;
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};

#[derive(Clone, Serialize, Deserialize)]
pub struct StdioServerParameters {
    pub command: String,
    pub args: Vec<String>,
    pub env: Option<HashMap<String, String>>,
}

#[derive(Clone, Serialize, Deserialize)]
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

#[derive(Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub content: Vec<Content>,
    #[serde(rename = "isError", default)]
    pub is_error: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ToolDescription {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

#[derive(Clone, Serialize, Deserialize)]
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
    pub is_zero_trust: bool,
}

impl ClientSession {
    pub async fn start(params: StdioServerParameters, is_zero_trust: bool) -> Result<Self> {
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
                        let mut line = match serde_json::to_string(&req) {
                            Ok(l) => l,
                            Err(_e) => {
                                continue;
                            }
                        };
                        line.push('\n');
                        if let Err(_e) = writer.write_all(line.as_bytes()).await {
                            break;
                        }
                    }
                    line_result = reader.next_line() => {
                        match line_result {
                            Ok(Some(line)) => {
                                if !line.trim().starts_with('{') {
                                    continue;
                                }
                                if let Ok(message) = serde_json::from_str::<Value>(&line)
                                    && let Some(id_val) = message.get("id")
                                        && let Some(id) = id_val.as_i64()
                                            && let Some(tx) = pending_requests.remove(&id) {
                                                if let Some(error) = message.get("error") {
                                                    let msg = error.get("message").and_then(|m| m.as_str()).unwrap_or("Unknown error");
                                                    let _ = tx.send(Err(anyhow!("{}", msg)));
                                                } else {
                                                    let result = message.get("result").cloned().unwrap_or(Value::Null);
                                                    let _ = tx.send(Ok(result));
                                                }
                                            }
                            }
                            Ok(None) => break,
                            Err(_e) => {
                                break;
                            }
                        }
                    }
                }
            }
        });

        Ok(Self {
            child,
            request_tx,
            is_zero_trust,
        })
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

use std::future::Future;
use std::pin::Pin;

pub type FastMcpToolFn =
    Box<dyn Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value>> + Send>> + Send + Sync>;

pub struct FastMcp {
    pub name: String,
    pub tools: HashMap<String, FastMcpToolFn>,
    pub zero_trust: bool,
}

impl FastMcp {
    pub fn new(name: &str, zero_trust: bool) -> Self {
        Self {
            name: name.to_string(),
            tools: HashMap::new(),
            zero_trust,
        }
    }

    pub fn tool<F, Fut>(&mut self, name: &str, func: F)
    where
        F: Fn(Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value>> + Send + 'static,
    {
        self.tools
            .insert(name.to_string(), Box::new(move |args| Box::pin(func(args))));
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

                        // Zero Trust Validation
                        let validation_error = if self.zero_trust {
                            let auth_token = arguments
                                .get("_meta")
                                .and_then(|m| m.get("auth_token"))
                                .and_then(|t| t.as_str());

                            if let Some(token_b64) = auth_token {
                                match base64::Engine::decode(
                                    &base64::engine::general_purpose::URL_SAFE_NO_PAD,
                                    token_b64,
                                ) {
                                    Ok(token_bytes) => {
                                        use crate::security::identity::IdentityManager;
                                        use crate::security::pqc_cose::HybridSigner;

                                        // 1. First pass: extract identity without full verification to get the client ID
                                        // In COSE, we can peek the payload or use a hint.
                                        // For simplicity, we use verify_hybrid_token with a dynamic fetcher.
                                        let identity = HybridSigner::verify_hybrid_token(
                                            &token_bytes,
                                            &[],        // Temporary empty classical key
                                            |_| vec![], // Temporary empty PQC key
                                        );

                                        // 2. Real verification: use the ID from the token to find registered keys
                                        if let Some(id_obj) = identity
                                            && let Some(client_id) =
                                                id_obj.get("sub").and_then(|s| s.as_str())
                                        {
                                            let classical_pub = IdentityManager::get_public_key_for(
                                                "clients",
                                                client_id,
                                                "id_ed25519.pub",
                                            );

                                            if let Ok(cpk) = classical_pub {
                                                let verified_identity =
                                                    HybridSigner::verify_hybrid_token(
                                                        &token_bytes,
                                                        &cpk,
                                                        |v| {
                                                            let filename = match v {
                                                                PQCVariant::MLDSA44 => {
                                                                    "id_mldsa44.pub"
                                                                }
                                                                PQCVariant::MLDSA65 => {
                                                                    "id_mldsa65.pub"
                                                                }
                                                                PQCVariant::MLDSA87 => {
                                                                    "id_mldsa87.pub"
                                                                }
                                                            };
                                                            IdentityManager::get_public_key_for(
                                                                "clients", client_id, filename,
                                                            )
                                                            .unwrap_or_default()
                                                        },
                                                    );

                                                if verified_identity.is_some() {
                                                    None // Success
                                                } else {
                                                    Some("Zero Trust: PQC signature verification failed for client".to_string())
                                                }
                                            } else {
                                                Some(format!(
                                                    "Zero Trust: Client '{}' is not registered (keys not found)",
                                                    client_id
                                                ))
                                            }
                                        } else {
                                            Some(
                                                "Zero Trust: Invalid or missing identity in token"
                                                    .to_string(),
                                            )
                                        }
                                    }
                                    Err(_) => {
                                        Some("Zero Trust: Invalid token encoding".to_string())
                                    }
                                }
                            } else {
                                Some("Zero Trust: Missing auth_token in _meta".to_string())
                            }
                        } else {
                            None
                        };

                        let result = if let Some(err_msg) = validation_error {
                            Err(err_msg)
                        } else if let Some(name) = tool_name {
                            if let Some(tool) = self.tools.get(name) {
                                match tool(arguments).await {
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
