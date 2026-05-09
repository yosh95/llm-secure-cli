use serde_json::{Value, json};
use std::io::{self, BufRead, Write};

fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    // Read from stdin line by line (MCP stdio protocol)
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let id = &request["id"];
        let method = request["method"].as_str().unwrap_or("");

        let response = match method {
            "initialize" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": {
                        "name": "rust-test-server",
                        "version": "0.1.0"
                    }
                }
            }),
            "tools/list" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "tools": [{
                        "name": "echo_tool",
                        "description": "A tool that echoes back the input message",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "message": { "type": "string", "description": "The message to repeat" }
                            },
                            "required": ["message"]
                        }
                    }]
                }
            }),
            "tools/call" => {
                let args = &request["params"]["arguments"];
                let msg = args["message"].as_str().unwrap_or("No message");
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [
                            {
                                "type": "text",
                                "text": format!("[Server Response] Echo: {}", msg)
                            }
                        ]
                    }
                })
            }
            // Metadata or notifications
            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {}
            }),
        };

        // Output as a single JSON line
        writeln!(stdout, "{}", response.to_string())?;
        stdout.flush()?;
    }
    Ok(())
}
