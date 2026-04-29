use colored::Colorize;
use llm_secure_cli::core::context::AppContext;
use llm_secure_cli::security::dual_llm_verifier::verify_tool_call;
use llm_secure_cli::security::identity::IdentityManager;
use llm_secure_cli::security::pqc::{MldsaVariant, MlkemVariant, PqcProvider};
use llm_secure_cli::security::static_analyzer::StaticAnalyzer;
use serde_json::json;
use std::sync::Arc;
use std::time::Instant;

fn section(title: &str) {
    println!("\n{}", "─".repeat(66));
    println!("  {}", title.bold());
    println!("{}", "─".repeat(66));
}

fn header(title: &str) {
    println!("\n{}", "═".repeat(66));
    println!("  {}", title.bold());
    println!("{}", "═".repeat(66));
}

fn timeit<F, T>(f: F, reps: u32) -> (f64, f64)
where
    F: Fn() -> T,
{
    let mut samples = Vec::new();
    for _ in 0..reps {
        let start = Instant::now();
        f();
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let mean = samples.iter().sum::<f64>() / reps as f64;
    let variance = samples.iter().map(|s| (s - mean).powi(2)).sum::<f64>() / reps as f64;
    (mean, variance.sqrt())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = Arc::new(AppContext::new());

    // Register clients
    {
        let mut registry = ctx.client_registry.lock().unwrap();
        registry.register(
            "openai",
            Arc::new(|model, stdout, raw, config_manager| {
                Box::new(llm_secure_cli::llm::providers::openai::OpenAiClient::new(
                    config_manager,
                    model,
                    stdout,
                    raw,
                ))
            }),
        );
        registry.register(
            "anthropic",
            Arc::new(|model, stdout, raw, config_manager| {
                Box::new(
                    llm_secure_cli::llm::providers::anthropic::ClaudeClient::new(
                        config_manager,
                        model,
                        stdout,
                        raw,
                    ),
                )
            }),
        );
        registry.register(
            "google",
            Arc::new(|model, stdout, raw, config_manager| {
                Box::new(llm_secure_cli::llm::providers::google::GeminiClient::new(
                    config_manager,
                    model,
                    stdout,
                    raw,
                ))
            }),
        );
    }

    header("Unified Security Framework — Comprehensive Benchmark (Rust)");

    // Phase 1
    section("Phase 1: Structural Guardrails (Space)");
    let code = "import os; os.system('rm -rf /')";
    let (mean, std) = timeit(|| StaticAnalyzer::is_obviously_malicious(code), 1000);
    println!(
        "Static Analysis (Pattern)  : {:.4} ms  (σ={:.4} ms, n=1000)",
        mean, std
    );

    let (base_mean, _) = timeit(
        || {
            let _ = std::process::Command::new("python3")
                .arg("-c")
                .arg("print('hello')")
                .output();
        },
        10,
    );
    println!("Base Subprocess Latency    : {:.2} ms  (n=10)", base_mean);

    // Phase 2
    section("Phase 2: Behavioral & Identity Assurance (Behavior)");
    IdentityManager::ensure_keys(false)?;
    let (gen_mean, gen_std) = timeit(
        || {
            IdentityManager::generate_token(
                &ctx.config_manager.get_config(),
                None,
                None,
                Some("ls"),
                Some(&json!({})),
            )
            .unwrap()
        },
        100,
    );
    println!(
        "Token Gen (ML-DSA-65)      : {:.2} ms  (σ={:.2} ms, n=100)",
        gen_mean, gen_std
    );

    // Phase 3
    section("Phase 3: Post-Quantum Resilience (Cryp. Agility)");
    println!(
        "  {:<12} | {:<12} | {:<12} | {:<12}",
        "Algorithm", "Keygen (ms)", "Sign (ms)", "Verify (ms)"
    );
    println!("  {:-<12}-+-{:-<12}-+-{:-<12}-+-{:-<12}", "", "", "", "");

    for variant in [
        MldsaVariant::Mldsa44,
        MldsaVariant::Mldsa65,
        MldsaVariant::Mldsa87,
    ] {
        let (kg_mean, _) = timeit(|| PqcProvider::generate_mldsa_keypair(variant), 10);
        let (pk, sk) = PqcProvider::generate_mldsa_keypair(variant);
        let msg = b"Verify Tool Execution Claim";
        let (s_mean, _) = timeit(|| PqcProvider::sign_mldsa(msg, &sk, variant), 100);
        let sig = PqcProvider::sign_mldsa(msg, &sk, variant).unwrap();
        let (v_mean, _) = timeit(|| PqcProvider::verify_mldsa(msg, &sig, &pk, variant), 100);
        println!(
            "  {:<12} | {:<12.2} | {:<12.2} | {:<12.2}",
            variant.to_str(),
            kg_mean,
            s_mean,
            v_mean
        );
    }

    println!(
        "\n  {:<12} | {:<12} | {:<12} | {:<12}",
        "Algorithm", "Keygen (ms)", "Encaps (ms)", "Decaps (ms)"
    );
    println!("  {:-<12}-+-{:-<12}-+-{:-<12}-+-{:-<12}", "", "", "", "");
    for variant in [
        MlkemVariant::Mlkem512,
        MlkemVariant::Mlkem768,
        MlkemVariant::Mlkem1024,
    ] {
        let (kg_mean, _) = timeit(|| PqcProvider::generate_mlkem_keypair(variant), 10);
        let (pk, sk) = PqcProvider::generate_mlkem_keypair(variant);
        let (e_mean, _) = timeit(|| PqcProvider::encapsulate_mlkem(&pk, variant), 100);
        let (_ss, ct) = PqcProvider::encapsulate_mlkem(&pk, variant);
        let (d_mean, _) = timeit(|| PqcProvider::decapsulate_mlkem(&ct, &sk, variant), 100);
        println!(
            "  {:<12} | {:<12.2} | {:<12.2} | {:<12.2}",
            variant.to_str(),
            kg_mean,
            e_mean,
            d_mean
        );
    }

    // Phase 4
    section("Phase 4: Intent Verification (Dual LLM)");
    let providers = [
        ("google", "lite"),
        ("openai", "nano"),
        ("anthropic", "haiku"),
    ];

    println!(
        "  {:<12} | {:<10} | {:<12}",
        "Provider", "Model", "Latency (ms)"
    );
    println!("  {:-<12}-+-{:-<10}-+-{:-<12}", "", "", "");

    for (provider, model) in providers {
        let prompt = "Read my todo list in todo.txt";
        let tool = "read_file_content";
        let args = json!({"path": "todo.txt", "explanation": "Reading requested file."});

        // Use set_config to update provider and model for verification
        {
            let mut config = ctx.config_manager.get_config();
            config.security.dual_llm_provider = provider.to_string();
            config.security.dual_llm_model = model.to_string();
            ctx.config_manager.set_config(config);
        }

        let has_key = ctx.config_manager.get_api_key(provider).is_some();
        if has_key {
            // Warm up
            let _ = verify_tool_call(
                ctx.clone(),
                prompt,
                tool,
                &args,
                None,
                &ctx.config_manager.get_config().security,
            )
            .await;

            let mut samples = Vec::new();
            for _ in 0..3 {
                let start = Instant::now();
                let _ = verify_tool_call(
                    ctx.clone(),
                    prompt,
                    tool,
                    &args,
                    None,
                    &ctx.config_manager.get_config().security,
                )
                .await;
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            let mean = samples.iter().sum::<f64>() / samples.len() as f64;
            println!("  {:<12} | {:<10} | {:>12.2}", provider, model, mean);
        } else {
            println!(
                "  {:<12} | {:<10} | {:>12}",
                provider, model, "N/A (No Key)"
            );
        }
    }

    header("Summary (Worst-Case Sequential)");
    println!("  See Table V in the technical report for final results.");

    Ok(())
}
