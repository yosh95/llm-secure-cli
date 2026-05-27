#![allow(clippy::unwrap_used, clippy::expect_used)]
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let res = run_client();
    if let Err(e) = res {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
    Ok(())
}

fn run_client() -> Result<(), Box<dyn std::error::Error>> {
    println!("Starting llsc in MCP server mode...");

    // Start llsc in server mode (via cargo run)
    let mut child = Command::new("cargo")
        .args(["run", "--", "--mcp-server"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null()) // Mute stderr to avoid mixing logs with JSON
        .spawn()
        .expect("Failed to start llsc");

    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let mut child_stdin = child.stdin.take().expect("Failed to open stdin");
        let mut child_stdout = BufReader::new(child.stdout.take().expect("Failed to open stdout"));

        // 1. Send 'initialize' request
        println!(">>> Sending 'initialize' request...");
        let init_req = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "manual-tester", "version": "1.0.0" }
            }
        });

        writeln!(child_stdin, "{}", init_req)?;
        child_stdin.flush()?;

        // 2. Read response
        let mut response_line = String::new();
        child_stdout.read_line(&mut response_line)?;

        if response_line.is_empty() {
            println!("Error: No response from llsc server.");
        } else {
            let resp: Value = serde_json::from_str(&response_line)?;
            println!("<<< Received response:");
            println!("{}", serde_json::to_string_pretty(&resp)?);

            if resp["result"]["serverInfo"]["name"].is_string() {
                println!("\nSUCCESS: llsc identified itself correctly.");
            } else {
                println!("\nFAILURE: Unexpected response format.");
            }
        }
        Ok(())
    })();

    // Ensure child is terminated and waited for
    let _ = child.kill();
    let _ = child.wait();

    result
}
