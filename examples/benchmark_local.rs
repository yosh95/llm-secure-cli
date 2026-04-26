use llm_secure_cli::security::pqc::{MldsaVariant, MlkemVariant, PqcProvider};
use llm_secure_cli::security::static_analyzer::StaticAnalyzer;
use std::time::Instant;

fn main() {
    println!("\n=== High-Assurance Primitives Benchmark (Local) ===");

    // 1. PQC Primitives (Primary Security Layer)
    println!("\n[1] Post-Quantum Cryptography (ML-DSA / ML-KEM)");
    for variant in [
        MldsaVariant::Mldsa44,
        MldsaVariant::Mldsa65,
        MldsaVariant::Mldsa87,
    ] {
        let name = variant.to_str();
        // ... (existing keygen/sign/verify logic remains, but clearly marked as primary)

        // Keygen
        let start = Instant::now();
        let (pk, sk) = PqcProvider::generate_mldsa_keypair(variant);
        let elapsed = start.elapsed();
        println!("{} Keygen: {:?}", name, elapsed);

        // Sign
        let msg = b"Hello, world!";
        let start = Instant::now();
        for _ in 0..100 {
            let _ = PqcProvider::sign_mldsa(msg, &sk, variant);
        }
        let elapsed = start.elapsed();
        let sig = PqcProvider::sign_mldsa(msg, &sk, variant).expect("Signing failed");
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
    let variant = MlkemVariant::Mlkem768;
    let (pk, sk) = PqcProvider::generate_mlkem_keypair(variant);

    // Encaps
    let start = Instant::now();
    for _ in 0..1000 {
        PqcProvider::encapsulate_mlkem(&pk, variant);
    }
    let elapsed = start.elapsed();
    let (_ss, ct) = PqcProvider::encapsulate_mlkem(&pk, variant);
    println!("ML-KEM-768 Encaps (1000 runs): {:?}", elapsed);
    println!("ML-KEM-768 Encaps (avg): {:?} per run", elapsed / 1000);

    // Decaps
    let start = Instant::now();
    for _ in 0..1000 {
        PqcProvider::decapsulate_mlkem(&ct, &sk, variant);
    }
    let elapsed = start.elapsed();
    println!("ML-KEM-768 Decaps (1000 runs): {:?}", elapsed);
    println!("ML-KEM-768 Decaps (avg): {:?} per run", elapsed / 1000);

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
}
