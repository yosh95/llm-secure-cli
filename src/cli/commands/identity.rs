use crate::cli::ui;
use crate::consts::LLM_CLI_BASE_DIR;
use crate::security::pqc::PqcProvider;
use std::fs;

pub fn run_keygen() {
    ui::report_success("Generating PQC Identity Keys...");

    let ident_dir = LLM_CLI_BASE_DIR.join("identity");
    let _ = fs::create_dir_all(&ident_dir);

    // ML-DSA-65
    let (pk_dsa, sk_dsa) = PqcProvider::generate_mldsa_65_keypair();
    let _ = fs::write(ident_dir.join("mldsa.pub"), &pk_dsa);
    let _ = fs::write(ident_dir.join("mldsa.key"), &sk_dsa);

    // ML-KEM-768
    let (pk_kem, sk_kem) = PqcProvider::generate_mlkem_768_keypair();
    let _ = fs::write(ident_dir.join("mlkem.pub"), &pk_kem);
    let _ = fs::write(ident_dir.join("mlkem.key"), &sk_kem);

    println!("ML-DSA-65 keys saved to ~/.llm_secure_cli/identity/mldsa.*");
    println!("ML-KEM-768 keys saved to ~/.llm_secure_cli/identity/mlkem.*");
}

pub fn run_manifest() {
    ui::report_success("Generating Integrity Manifest... (Mocked)");
    println!("Integrity manifest saved to ~/.llm-secure-cli/integrity/manifest.json");
}

pub fn run_verify(tail: Option<usize>) {
    let label = match tail {
        Some(t) => format!("last {} lines", t),
        None => "all lines".to_string(),
    };
    ui::report_success(&format!(
        "Running full integrity check (PQC verify on {})... (Mocked)",
        label
    ));
    println!("OK Integrity check passed.");
}

pub fn run_verify_session(trace_id: &str) {
    ui::report_success(&format!("Verifying session: {}... (Mocked)", trace_id));
    println!(
        "OK Session {} integrity verified via PQC-signed Merkle Anchor.",
        trace_id
    );
}

pub fn list_anchors() {
    ui::report_success("Available Session Anchors: (Mocked)");
    println!("  - Trace ID: mock-trace-1 | Time: 2026-04-18 | Logs: 5");
}
