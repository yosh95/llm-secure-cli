use llm_secure_cli::security::pqc::{MldsaVariant, MlkemVariant, PqcProvider};
use llm_secure_cli::security::static_analyzer::StaticAnalyzer;
use std::time::Instant;

#[test]
fn benchmark_local_primitives() {
    println!("\n=== Rust Local Primitives Benchmark ===");

    // 1. AST Analysis
    let code = "import os; os.system('ls')";
    let start = Instant::now();
    for _ in 0..1000 {
        StaticAnalyzer::analyze_python_safety(code);
    }
    let elapsed = start.elapsed();
    println!("AST Analysis (1000 runs): {:?}", elapsed);
    println!("AST Analysis (avg): {:?} per run", elapsed / 1000);

    // 2. ML-DSA Keygen/Sign/Verify
    for variant in [
        MldsaVariant::Mldsa44,
        MldsaVariant::Mldsa65,
        MldsaVariant::Mldsa87,
    ] {
        let name = variant.to_str();

        // Keygen
        let start = Instant::now();
        let (pk, sk) = PqcProvider::generate_mldsa_keypair(variant);
        let elapsed = start.elapsed();
        println!("{} Keygen: {:?}", name, elapsed);

        // Sign
        let msg = b"Hello, world!";
        let start = Instant::now();
        for _ in 0..100 {
            PqcProvider::sign_mldsa(msg, &sk, variant);
        }
        let elapsed = start.elapsed();
        let sig = PqcProvider::sign_mldsa(msg, &sk, variant);
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
    let (ss, ct) = PqcProvider::encapsulate_mlkem(&pk, variant);
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
}
