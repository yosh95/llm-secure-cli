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
    let ctx = Arc::new(AppContext::default());

    // Register clients
    {
        use llm_secure_cli::llm::providers::openai_compatible::OpenAiCompatibleClient;
        let mut registry = ctx.client_registry.lock().await;
        for provider in ["openai", "anthropic", "google"] {
            let p_name = provider.to_string();
            let closure_p_name = p_name.clone();
            registry.register(
                &p_name,
                Arc::new(move |model, stdout, raw, config_manager| {
                    let api_url = config_manager
                        .get_config()?
                        .providers
                        .get(&closure_p_name)
                        .and_then(|p| p.api_url.clone())
                        .unwrap_or_else(|| match closure_p_name.as_str() {
                            "openai" => "https://api.openai.com/v1".to_string(),
                            "anthropic" => "https://api.anthropic.com/v1".to_string(),
                            "google" => "https://generativelanguage.googleapis.com/v1beta/openai".to_string(),
                            _ => "".to_string(),
                        });
                    let api_key = config_manager.get_api_key(&closure_p_name).unwrap_or_default();
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
    IdentityManager::ensure_keys()?;
    let (gen_mean, gen_std) = timeit(
        || {
            IdentityManager::generate_token(
                Some("ls"),
            )
            .unwrap()
        },
        100,
    );
    println!(
        "Token Gen (ML-DSA-87)      : {:.2} ms  (σ={:.2} ms, n=100)",
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
        MldsaVariant::MLDSA87,
    ] {
        let (kg_mean, _) = timeit(|| PqcProvider::generate_keypair(variant).unwrap(), 10);
        let (pk, sk) = PqcProvider::generate_keypair(variant).unwrap();
        let msg = b"Verify Tool Execution Claim";
        let (s_mean, _) = timeit(|| PqcProvider::sign(variant, &sk, msg).unwrap(), 100);
        let sig = PqcProvider::sign(variant, &sk, msg).unwrap();
        let (v_mean, _) = timeit(|| PqcProvider::verify(variant, &pk, msg, &sig), 100);
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
        MlkemVariant::MLKEM1024,
    ] {
        let (kg_mean, _) = timeit(|| PqcProvider::generate_kem_keypair(variant).unwrap(), 10);
        let (pk, sk) = PqcProvider::generate_kem_keypair(variant).unwrap();
        let (e_mean, _) = timeit(|| PqcProvider::encapsulate(variant, &pk).unwrap(), 100);
        let (_ss, ct) = PqcProvider::encapsulate(variant, &pk).unwrap();
        let (d_mean, _) = timeit(|| PqcProvider::decapsulate(variant, &ct, &sk).unwrap(), 100);
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
        let tool = "read_file";
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
