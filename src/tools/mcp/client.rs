use anyhow::Result;
use rmcp::{
    model::{CallToolRequestParams, CallToolResult, Tool as ToolDescription},
    service::{RoleClient, RunningService, ServiceExt},
    transport::{
        TokioChildProcess,
        streamable_http_client::{
            StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
        },
    },
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use tokio::process::Command;

#[derive(Clone, Serialize, Deserialize)]
pub struct StdioServerParameters {
    pub command: String,
    pub args: Vec<String>,
    pub env: Option<HashMap<String, String>>,
}

/// Wraps an rmcp client connection to an MCP server.
pub struct ClientSession {
    /// The underlying rmcp running service (client role).
    service: RunningService<RoleClient, ()>,
    /// Cached tool list after initialization.
    tools_cache: Option<Vec<ToolDescription>>,
    /// Human-readable server name.
    server_name: String,
}

impl ClientSession {
    /// Start a session with an MCP server using stdio transport.
    pub async fn start_stdio(params: StdioServerParameters) -> Result<Self> {
        let mut cmd = Command::new(&params.command);
        cmd.args(&params.args);

        if let Some(env) = params.env {
            cmd.envs(env);
        }

        let transport = TokioChildProcess::new(cmd)?;
        let service = ().serve(transport).await?;

        Ok(Self {
            service,
            tools_cache: None,
            server_name: params.command.clone(),
        })
    }

    /// Start a session with an MCP server using Streamable HTTP transport.
    pub async fn start_http(api_url: &str) -> Result<Self> {
        let config = StreamableHttpClientTransportConfig::with_uri(api_url);
        let transport = StreamableHttpClientTransport::with_client(reqwest::Client::new(), config);
        let service = ().serve(transport).await?;

        Ok(Self {
            service,
            tools_cache: None,
            server_name: api_url.to_string(),
        })
    }

    /// List tools from the server, using cache if available.
    pub async fn list_tools(&mut self) -> Result<Vec<ToolDescription>> {
        if let Some(ref cached) = self.tools_cache {
            return Ok(cached.clone());
        }

        // Use the built-in list_all_tools which handles pagination
        let all_tools = self.service.list_all_tools().await?;

        self.tools_cache = Some(all_tools.clone());
        Ok(all_tools)
    }

    /// Call a tool on the server.
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<CallToolResult> {
        let args_map: serde_json::Map<String, Value> =
            arguments.as_object().cloned().unwrap_or_default();

        let name_owned = name.to_string();
        let params = if args_map.is_empty() {
            CallToolRequestParams::new(name_owned)
        } else {
            CallToolRequestParams::new(name_owned).with_arguments(args_map)
        };

        let result = self.service.call_tool(params).await?;
        Ok(result)
    }

    pub fn server_name(&self) -> &str {
        &self.server_name
    }
}
