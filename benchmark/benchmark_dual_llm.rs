use colored::Colorize;
use llm_secure_cli::core::context::AppContext;
use llm_secure_cli::llm::providers::openai_compatible::OpenAiCompatibleClient;
use llm_secure_cli::security::dual_llm_verifier::verify_tool_call_full;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Serialize, Deserialize)]
struct Scenario {
    label: String,
    intent: String,
    tool: String,
    arguments: Value,
    expected: String,
}

use llm_secure_cli::cli::ui::CliUi;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!(
        "{}",
        "=== Dual LLM Verification Benchmark ===".bold().cyan()
    );

    let ctx = Arc::new(AppContext::new(Arc::new(CliUi)));

    // Register clients
    {
        let mut registry = ctx.client_registry.lock().await;
        for provider in ["ollama", "openrouter"] {
            let p_name = provider.to_string();
            let closure_p_name = p_name.clone();
            registry.register(
                &p_name,
                Arc::new(move |model, stdout, raw, config_manager| {
                    let api_url = config_manager
                        .get_config()
                        .unwrap()
                        .providers
                        .get(&closure_p_name)
                        .and_then(|p| p.api_url.clone())
                        .unwrap_or_else(|| match closure_p_name.as_str() {
                            "ollama" => "http://localhost:11434/v1".to_string(),
                            "openrouter" => "https://openrouter.ai/api/v1".to_string(),
                            _ => "".to_string(),
                        });
                    let api_key = config_manager
                        .get_api_key(&closure_p_name)
                        .unwrap_or_default();
                    OpenAiCompatibleClient::builder(config_manager)
                        .provider_name(&closure_p_name)
                        .api_url(&api_url)
                        .api_key(&api_key)
                        .model(model)
                        .stdout(stdout)
                        .raw(raw)
                        .build()
                        .map(|c| Box::new(c) as _)
                }),
            );
        }
    }

    // Load scenarios and arguments
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!(
            "Usage: cargo bench --bench benchmark_dual_llm -- <provider> <model> [json_path]"
        );
        eprintln!("Example: cargo bench --bench benchmark_dual_llm -- ollama llama3");
        eprintln!(
            "Example: cargo bench --bench benchmark_dual_llm -- openrouter anthropic/claude-3-haiku"
        );
        std::process::exit(1);
    }

    let (target_provider, target_model) = match (args.get(1), args.get(2)) {
        (Some(p), Some(m)) => (p, m),
        _ => {
            eprintln!("{}: Missing arguments.", "Error".red().bold());
            eprintln!(
                "Usage: cargo bench --bench benchmark_dual_llm -- <provider> <model> [json_path]"
            );
            std::process::exit(1);
        }
    };

    let json_path = args
        .get(3)
        .map(|s| s.as_str())
        .unwrap_or("benchmark/scenarios.json");

    println!("Reading scenarios from: {}", json_path.cyan());
    let scenarios_json = fs::read_to_string(json_path)?;
    let scenarios: Vec<Scenario> = serde_json::from_str(&scenarios_json)?;
    println!("Loaded {} scenarios from {}\n", scenarios.len(), json_path);

    let providers = vec![(target_provider.as_str(), target_model.as_str())];

    let security_config = ctx.config_manager.get_config().unwrap().security.clone();

    for (p_alias, p_model) in providers {
        let api_key = ctx.config_manager.get_api_key(p_alias);

        // Validation for provider specific requirements
        if p_alias == "openrouter" && api_key.is_none() {
            println!("{}", "Error: OpenRouter requires OPENROUTER_API_KEY in environment or ~/.llm_secure_cli/.env".red());
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
            use llm_secure_cli::security::dual_llm_verifier::VerificationOutcome;
            use llm_secure_cli::security::dual_llm_verifier::VerificationParams;
            let outcome = verify_tool_call_full(VerificationParams {
                ctx_app: ctx.clone(),
                user_query: &scenario.intent,
                tool_name: &scenario.tool,
                tool_args: &scenario.arguments,
                context: None,
                config: &security_config,
                provider: Some(p_alias.to_string()),
                model: Some(p_model.to_string()),
            })
            .await;
            let (safe, reason) = match outcome {
                VerificationOutcome::Allowed(r) => (true, r),
                VerificationOutcome::Modified(_, r) => (true, r),
                VerificationOutcome::Rejected(r) => (false, r),
                VerificationOutcome::FallbackRequired(r) => (false, r),
            };
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
