use colored::Colorize;
use llm_secure_cli::llm::providers::anthropic::ClaudeClient;
use llm_secure_cli::llm::providers::google::GeminiClient;
use llm_secure_cli::llm::providers::ollama::OllamaClient;
use llm_secure_cli::llm::providers::openai::OpenAiClient;
use llm_secure_cli::security::dual_llm_verifier::verify_tool_call_full;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::time::Instant;

#[derive(Debug, Serialize, Deserialize)]
struct Scenario {
    label: String,
    intent: String,
    tool: String,
    arguments: Value,
    expected: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!(
        "{}",
        "=== Dual LLM Verification Benchmark ===".bold().cyan()
    );

    // Register clients
    {
        let mut registry = llm_secure_cli::llm::registry::CLIENT_REGISTRY
            .lock()
            .unwrap();
        registry.register("openai", |model, stdout, raw| {
            Box::new(OpenAiClient::new(model, stdout, raw))
        });
        registry.register("anthropic", |model, stdout, raw| {
            Box::new(ClaudeClient::new(model, stdout, raw))
        });
        registry.register("ollama", |model, stdout, raw| {
            Box::new(OllamaClient::new(model, stdout, raw))
        });
        registry.register("google", |model, stdout, raw| {
            Box::new(GeminiClient::new(model, stdout, raw))
        });
    }

    // Load scenarios from JSON
    let args: Vec<String> = std::env::args().collect();
    let json_path = args
        .get(1)
        .map(|s| s.as_str())
        .unwrap_or("benchmark/scenarios.json");

    let target_provider = args.get(2).map(|s| s.as_str());
    let target_model = args.get(3).map(|s| s.as_str());

    println!("Reading scenarios from: {}", json_path.cyan());
    let scenarios_json = fs::read_to_string(json_path)?;
    let scenarios: Vec<Scenario> = serde_json::from_str(&scenarios_json)?;
    println!("Loaded {} scenarios from {}\n", scenarios.len(), json_path);

    let providers = if let (Some(p), Some(m)) = (target_provider, target_model) {
        vec![(p, m)]
    } else {
        vec![
            ("google", "lite"),
            ("openai", "mini"),
            ("anthropic", "haiku"),
        ]
    };

    let security_config = llm_secure_cli::config::CONFIG_MANAGER.get_config().security;

    for (p_alias, p_model) in providers {
        let has_key = llm_secure_cli::config::CONFIG_MANAGER
            .get_api_key(p_alias)
            .is_some();
        if !has_key {
            println!("Skipping {} (No API Key found)", p_alias.yellow());
            continue;
        }

        println!(
            "\n{}",
            format!("Testing Provider: {} ({})", p_alias, p_model)
                .bold()
                .green()
        );
        println!("{:-<100}", "");

        let mut correct = 0;
        let mut total_time = 0;
        let mut tp = 0; // True Positive (Blocked Malicious)
        let mut tn = 0; // True Negative (Allowed Safe)
        let mut fp = 0; // False Positive (Blocked Safe)
        let mut fn_ = 0; // False Negative (Allowed Malicious)

        for scenario in &scenarios {
            let start = Instant::now();
            let (safe, reason) = verify_tool_call_full(
                &scenario.intent,
                &scenario.tool,
                &scenario.arguments,
                None,
                &security_config,
                Some(p_alias.to_string()),
                Some(p_model.to_string()),
            )
            .await;
            let elapsed = start.elapsed().as_millis();
            total_time += elapsed;

            let actual = if safe { "SAFE" } else { "BLOCKED" };
            let is_correct = actual == scenario.expected;

            if is_correct {
                correct += 1;
                if safe {
                    tn += 1;
                } else {
                    tp += 1;
                }
            } else {
                if safe {
                    fn_ += 1;
                } else {
                    fp += 1;
                }
            }

            println!(
                "[{:<30}] Expected: {:<7} | Actual: {:<7} | Time: {:>4}ms | Result: {}",
                if scenario.label.len() > 30 {
                    scenario.label[..27].to_string() + "..."
                } else {
                    scenario.label.clone()
                },
                scenario.expected,
                if actual == "SAFE" {
                    actual.blue()
                } else {
                    actual.red()
                },
                elapsed,
                if is_correct {
                    "PASS".green()
                } else {
                    "FAIL".red()
                }
            );

            if !is_correct {
                println!("  {} {}", "Reason:".dimmed(), reason.dimmed());
            }
        }

        let total = scenarios.len() as f64;
        println!("{:-<100}", "");
        println!("Summary for {}:", p_alias.bold());
        println!(
            "  Accuracy : {:.2}% ({}/{})",
            (correct as f64 / total) * 100.0,
            correct,
            total
        );
        println!("  Avg Latency: {} ms", total_time / scenarios.len() as u128);
        println!(
            "  Confusion Matrix: TP={}, TN={}, FP={}, FN={}",
            tp, tn, fp, fn_
        );

        let precision = if tp + fp > 0 {
            tp as f64 / (tp + fp) as f64
        } else {
            0.0
        };
        let recall = if tp + fn_ > 0 {
            tp as f64 / (tp + fn_) as f64
        } else {
            0.0
        };
        let f1 = if precision + recall > 0.0 {
            2.0 * precision * recall / (precision + recall)
        } else {
            0.0
        };

        println!(
            "  Precision: {:.4}, Recall: {:.4}, F1: {:.4}",
            precision, recall, f1
        );
    }

    Ok(())
}
