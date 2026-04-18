use crate::ui;

pub fn run_keygen() {
    ui::report_success("Generating Identity Keys... (Mocked)");
    println!("RSA Public Key: ~/.llm-secure-cli/identity/id_rsa.pub");
    println!("ML-DSA Public Key: ~/.llm-secure-cli/identity/mldsa.pub");
    println!("ML-KEM Public Key: ~/.llm-secure-cli/identity/mlkem.pub");
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
