use crate::cli::ui;
use crate::security::dual_llm_verifier::verify_tool_call_full;
use colored::Colorize;
use serde_json::json;
use std::io::{self, Write};
use std::time::Instant;

pub async fn run_benchmark(provider: &str, model: &str, iterations: u32) {
    ui::report_success("Benchmarking Dual LLM Verification...");
    println!("Provider: {}", provider);
    println!("Model:    {}", model);

    let user_prompt = "Write a python script to list files in /etc";
    let tool_name = "execute_command";
    let args = json!({
        "command": "ls",
        "args": ["/etc"]
    });

    let mut latencies = Vec::new();

    for i in 0..iterations {
        print!("  Iteration {}/{}... ", i + 1, iterations);
        io::stdout().flush().unwrap();

        let start = Instant::now();
        let (safe, reason) = verify_tool_call_full(
            user_prompt,
            tool_name,
            &args,
            None,
            Some(provider.to_string()),
            Some(model.to_string()),
        )
        .await;
        let elapsed = start.elapsed().as_secs_f64();

        if reason.to_lowercase().contains("error") || reason.to_lowercase().contains("failed") {
            println!("\n[red]  Iteration {} failed: {}[/red]", i + 1, reason);
        } else {
            latencies.push(elapsed);
            println!("{:.2}s (safe: {})", elapsed, safe);
        }
    }

    if latencies.is_empty() {
        ui::report_error("Benchmark failed: No successful requests.");
        return;
    }

    let avg_latency = latencies.iter().sum::<f64>() / latencies.len() as f64;
    let min_latency = latencies.iter().fold(f64::INFINITY, |a, &b| a.min(b));
    let max_latency = latencies.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));

    println!("\n{}", "SUCCESS Benchmark Results:".bright_green().bold());
    println!("  Average Latency: {:.2}s", avg_latency);
    println!("  Min Latency:     {:.2}s", min_latency);
    println!("  Max Latency:     {:.2}s", max_latency);

    if avg_latency > 2.0 {
        ui::report_warning("Latency is high (>2s). Consider using a faster model.");
    }
}
