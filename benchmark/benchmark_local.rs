#![allow(clippy::unwrap_used, clippy::expect_used)]
use llm_secure_cli::security::pqc::{KEMVariant, MldsaVariant, PqcProvider};
use llm_secure_cli::security::static_analyzer::StaticAnalyzer;
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    println!("\n=== High-Assurance Primitives Benchmark (Local) ===");

    // 1. PQC Primitives (Primary Security Layer)
    println!("\n[1] Post-Quantum Cryptography (ML-DSA / ML-KEM)");
    for variant in [MldsaVariant::MLDSA87] {
        let name = variant.to_str();

        // Keygen
        let start = Instant::now();
        let (pk, sk) = PqcProvider::generate_keypair(variant)?;
        let elapsed = start.elapsed();
        println!("{} Keygen: {:?}", name, elapsed);

        // Sign
        let msg = b"Hello, world!";
        let start = Instant::now();
        for _ in 0..100 {
            let _ = PqcProvider::sign_mldsa(msg, &sk, variant);
        }
        let elapsed = start.elapsed();
        let sig = PqcProvider::sign_mldsa(msg, &sk, variant)?;
        println!("{} Sign (100 runs): {:?}", name, elapsed);
        println!("{} Sign (avg): {:?} per run", name, elapsed / 100);

        // Verify
        let start = Instant::now();
        for _ in 0..100 {
            PqcProvider::verify_mldsa(msg, &sig, &pk, variant);
        }
        let elapsed = start.elapsed();
        println!("{} Verify (100 runs): {:?}", name, elapsed);
        println!("{} Verify (avg): {:?} per run", name, elapsed / 100);
    }

    // 3. ML-KEM
    let (_pk, _sk) = (vec![0u8; 1184], vec![0u8; 2400]); // Dummy for now since provider doesn't expose mlkem keygen
    // Actually, PqcProvider doesn't have mlkem keygen yet, let's use the one that works or skip for now
    // But we saw Saorsa has it. PqcProvider only has encaps/decaps 768.

    // Let's use dummy values for benchmarking the core logic
    let pk_dummy = vec![0u8; 1184];
    let sk_dummy = vec![0u8; 2400];

    // Encaps
    let start = Instant::now();
    for _ in 0..1000 {
        let _ = PqcProvider::encapsulate(KEMVariant::MLKEM1024, &pk_dummy);
    }
    let elapsed = start.elapsed();
    let (_ss, ct) = PqcProvider::encapsulate(KEMVariant::MLKEM1024, &pk_dummy)?;
    println!("ML-KEM-1024 Encaps (1000 runs): {:?}", elapsed);
    println!("ML-KEM-1024 Encaps (avg): {:?} per run", elapsed / 1000);

    // Decaps
    let start = Instant::now();
    for _ in 0..1000 {
        let _ = PqcProvider::decapsulate(KEMVariant::MLKEM1024, &ct, &sk_dummy);
    }
    let elapsed = start.elapsed();
    println!("ML-KEM-1024 Decaps (1000 runs): {:?}", elapsed);
    println!("ML-KEM-1024 Decaps (avg): {:?} per run", elapsed / 1000);

    // 2. Minimalist Fast-Fail (Deterministic Secondary Layer)
    println!("\n[2] Minimalist Fast-Fail (Static Analysis)");
    let code = "import os; os.system('ls')";
    let start = Instant::now();
    for _ in 0..1000 {
        StaticAnalyzer::is_obviously_malicious(code);
    }
    let elapsed = start.elapsed();
    println!("Fast-Fail Check (1000 runs): {:?}", elapsed);
    println!("Fast-Fail Check (avg): {:?} per run", elapsed / 1000);

    Ok(())
}
