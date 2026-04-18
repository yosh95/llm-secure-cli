use crate::cli::ui;
use crate::consts::KEY_DIR;
use crate::security::identity::IdentityManager;
use crate::security::integrity::IntegrityVerifier;
use crate::security::merkle_anchor::SessionAnchorManager;
use std::fs;

pub fn run_keygen() {
    ui::report_success("Generating Secure Identity Keys (RSA + Post-Quantum)...");

    match IdentityManager::ensure_keys(true) {
        Ok(_) => {
            println!(
                "Keys successfully generated and stored in {}",
                KEY_DIR.display()
            );
            println!("- RSA (3072-bit)");
            println!("- ML-DSA-44, 65, 87 (Post-Quantum Signatures)");
            println!("- ML-KEM-768 (Post-Quantum Key Encapsulation)");
        }
        Err(e) => {
            ui::report_error(&format!("Failed to generate keys: {}", e));
        }
    }
}

pub fn run_manifest() {
    ui::report_success("Generating Integrity Manifest...");
    let verifier = IntegrityVerifier::new();
    match verifier.rebuild_manifest() {
        Ok(_) => {
            ui::report_success(&format!(
                "Integrity manifest saved to {}",
                verifier.manifest_path.display()
            ));
        }
        Err(e) => {
            ui::report_error(&format!("Failed to generate manifest: {}", e));
        }
    }
}

pub fn run_verify(_tail: Option<usize>) {
    ui::report_success("Running full integrity check...");
    let verifier = IntegrityVerifier::new();
    match verifier.verify() {
        Ok(true) => {
            ui::report_success("OK: System integrity verified.");
        }
        Ok(false) => {
            ui::report_error(
                "FAILED: System integrity failure detected (Binary or Config mismatch).",
            );
        }
        Err(e) => {
            ui::report_error(&format!("ERROR: Verification error: {}", e));
        }
    }
}

pub fn run_verify_session(trace_id: &str) {
    ui::report_success(&format!("Verifying session integrity: {}...", trace_id));
    match SessionAnchorManager::verify_session(trace_id) {
        Ok(true) => {
            ui::report_success(&format!(
                "OK: Session {} integrity verified via PQC-signed Merkle Anchor.",
                trace_id
            ));
        }
        Ok(false) => {
            ui::report_error(&format!(
                "FAILED: Session {} integrity verification failed or anchor not found.",
                trace_id
            ));
        }
        Err(e) => {
            ui::report_error(&format!("ERROR: Verification error: {}", e));
        }
    }
}

pub fn list_anchors() {
    ui::report_success("Available Session Anchors:");

    let anchor_dir = crate::consts::AUDIT_LOG_PATH
        .parent()
        .unwrap()
        .join("anchors");
    if !anchor_dir.exists() {
        println!("No session anchors found.");
        return;
    }

    if let Ok(entries) = fs::read_dir(anchor_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(anchor) = serde_json::from_str::<serde_json::Value>(&content) {
                        let trace_id = anchor
                            .get("trace_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        let count = anchor
                            .get("entry_count")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        let time = anchor
                            .get("timestamp")
                            .and_then(|v| v.as_str())
                            .unwrap_or("-");
                        println!(
                            "  - Trace ID: {} | Time: {} | Logs: {}",
                            trace_id, time, count
                        );
                    }
                }
            }
        }
    }
}
