#![allow(clippy::unwrap_used, clippy::expect_used)]
use llm_secure_cli::security::integrity::IntegrityVerifier;
use std::sync::Mutex;

static INTEGRITY_TEST_MUTEX: Mutex<()> = Mutex::new(());

#[test]
fn test_integrity_verifier_initialization() {
    let _guard = INTEGRITY_TEST_MUTEX.lock().expect("Lock poisoned");

    // IntegrityVerifier should initialize with a manifest path relative to base dir.
    let verifier = IntegrityVerifier::new();

    // Check if the path ends with the correct filename.
    assert!(verifier.manifest_path.ends_with("integrity_manifest.json"));
}

#[test]
fn test_verify_fails_gracefully_when_no_manifest() {
    let _guard = INTEGRITY_TEST_MUTEX.lock().expect("Lock poisoned");

    let mut verifier = IntegrityVerifier::new();
    // Point to a guaranteed non-existent file
    verifier.manifest_path = std::path::PathBuf::from("/non/existent/path/manifest.json");

    let result = verifier.verify();

    // Should return Err instead of panicking
    assert!(result.is_err());
    if let Err(e) = result {
        assert!(e.to_string().contains("not found"));
    }
}
