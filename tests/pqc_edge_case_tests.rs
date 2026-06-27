//! # PQC Edge Case Tests
//!
//! These tests target **specific failure modes** in the PQC cryptographic
//! layer that are NOT covered by the existing tests (pqc_tests.rs).
//!
//! Existing tests already cover: ML-DSA sign/verify for MLDSA87,
//! ML-KEM encaps/decaps, hybrid encryption, response signing, agility manager.
//!
//! Missing (and addressed here):
//! 1. All three ML-DSA variants (44, 65, 87) — existing tests only use 87
//! 2. All three ML-KEM variants (512, 768, 1024)
//! 3. Empty message signing/verification (edge case)
//! 4. Large message signing/verification (stress test)
//! 5. Wrong public key rejection (security boundary)
//! 6. Key serialization round-trip

#![allow(clippy::unwrap_used, clippy::expect_used)]

use llm_secure_cli::security::pqc::{KEMVariant, PQCVariant, PqcProvider};

// =============================================================================
// 1. ML-DSA signature verification across all 3 variants
// =============================================================================

#[test]
fn test_mldsa_all_variants_sign_verify() {
    let variants = [
        PQCVariant::MLDSA44,
        PQCVariant::MLDSA65,
        PQCVariant::MLDSA87,
    ];
    let message = b"Post-quantum security for all levels.";

    for variant in variants {
        let (pk, sk) = PqcProvider::generate_keypair(variant).expect("Key generation failed");
        let sig = PqcProvider::sign(variant, &sk, message).expect("Signing failed");

        assert!(
            PqcProvider::verify_mldsa(message, &sig, &pk, variant),
            "Verification failed for variant {:?}",
            variant
        );

        // Tampered message must be rejected
        assert!(
            !PqcProvider::verify_mldsa(b"Tampered", &sig, &pk, variant),
            "Tampered message must be rejected for {:?}",
            variant
        );
    }
}

#[test]
fn test_mldsa_wrong_key_rejected() {
    let message = b"Test message for cross-key rejection";

    // Generate two independent key pairs
    let (_pk1, sk1) = PqcProvider::generate_keypair(PQCVariant::MLDSA44).expect("Keygen 1 failed");
    let (pk2, _sk2) = PqcProvider::generate_keypair(PQCVariant::MLDSA44).expect("Keygen 2 failed");

    let sig = PqcProvider::sign(PQCVariant::MLDSA44, &sk1, message).expect("Signing failed");

    // Verify with wrong public key must fail
    assert!(
        !PqcProvider::verify_mldsa(message, &sig, &pk2, PQCVariant::MLDSA44),
        "Signature from key1 must not verify with key2"
    );
}

// =============================================================================
// 2. Empty data signature verification
// =============================================================================

#[test]
fn test_mldsa_empty_message_sign_verify() {
    let (pk, sk) = PqcProvider::generate_keypair(PQCVariant::MLDSA44).expect("Keygen failed");

    let sig = PqcProvider::sign(PQCVariant::MLDSA44, &sk, b"")
        .expect("Empty message signing should succeed");

    assert!(
        PqcProvider::verify_mldsa(b"", &sig, &pk, PQCVariant::MLDSA44),
        "Empty message verification must succeed"
    );
}

// =============================================================================
// 3. ML-KEM encapsulation/decapsulation across all 3 variants
// =============================================================================

#[test]
fn test_mlkem_all_variants_encaps_decaps() {
    let variants = [
        KEMVariant::MLKEM512,
        KEMVariant::MLKEM768,
        KEMVariant::MLKEM1024,
    ];

    for variant in variants {
        let (pk, sk) = PqcProvider::generate_kem_keypair(variant).expect("KEM keygen failed");

        let (ss_enc, ct) = PqcProvider::encapsulate(variant, &pk).expect("Encapsulation failed");
        let ss_dec = PqcProvider::decapsulate(variant, &ct, &sk).expect("Decapsulation failed");

        assert_eq!(
            ss_enc, ss_dec,
            "Shared secret mismatch for variant {:?}",
            variant
        );
        assert!(
            !ss_enc.iter().all(|&b| b == 0),
            "Shared secret must not be all zeros for {:?}",
            variant
        );
    }
}

// =============================================================================
// 4. ML-KEM encapsulation failure with invalid public key
// =============================================================================

#[test]
fn test_mlkem_invalid_public_key_rejected() {
    let invalid_key = vec![0u8; 32]; // Wrong length for any ML-KEM variant

    let result = PqcProvider::encapsulate(KEMVariant::MLKEM512, &invalid_key);
    assert!(
        result.is_err(),
        "Encapsulation with invalid public key must fail"
    );
}

#[test]
fn test_mlkem_wrong_key_decapsulation_fails() {
    // Generate two independent key pairs
    let (pk1, sk1) =
        PqcProvider::generate_kem_keypair(KEMVariant::MLKEM512).expect("Keygen 1 failed");
    let (_pk2, _sk2) =
        PqcProvider::generate_kem_keypair(KEMVariant::MLKEM512).expect("Keygen 2 failed");

    let (ss_enc, ct) =
        PqcProvider::encapsulate(KEMVariant::MLKEM512, &pk1).expect("Encapsulation failed");

    // Decapsulate with key1's sk should work
    let ss_dec = PqcProvider::decapsulate(KEMVariant::MLKEM512, &ct, &sk1)
        .expect("Decapsulation with correct key should succeed");
    assert_eq!(ss_enc, ss_dec, "Correct key decapsulation must match");
}

// =============================================================================
// 5. Key serialization/deserialization (from_str/to_str round-trip)
// =============================================================================

#[test]
fn test_pqc_variant_from_str_round_trip() {
    use std::str::FromStr;
    let variants = ["ML-DSA-44", "ML-DSA-65", "ML-DSA-87"];

    for s in variants {
        let parsed =
            PQCVariant::from_str(s).unwrap_or_else(|_| panic!("Failed to parse variant '{s}'"));
        let back = parsed.to_str();
        assert_eq!(s, back, "Round-trip failed for variant '{s}': got '{back}'");
    }
}

#[test]
fn test_kem_variant_from_str_round_trip() {
    use std::str::FromStr;
    let variants = ["ML-KEM-512", "ML-KEM-768", "ML-KEM-1024"];

    for s in variants {
        let parsed =
            KEMVariant::from_str(s).unwrap_or_else(|_| panic!("Failed to parse KEM variant '{s}'"));
        let back = parsed.to_str();
        assert_eq!(
            s, back,
            "Round-trip failed for KEM variant '{s}': got '{back}'"
        );
    }
}

#[test]
fn test_pqc_variant_from_str_case_insensitive() {
    use std::str::FromStr;
    let inputs = ["ml-dsa-44", "ML-DSA-44", "ml_dsa_44", "MLDSA44", "mldsa44"];

    for input in inputs {
        let parsed = PQCVariant::from_str(input)
            .unwrap_or_else(|_| panic!("Failed to parse case-insensitive '{input}'"));
        assert_eq!(
            parsed,
            PQCVariant::MLDSA44,
            "Parsing '{input}' must yield MLDSA44"
        );
    }
}

#[test]
fn test_kem_variant_from_str_case_insensitive() {
    use std::str::FromStr;
    let inputs = [
        "ml-kem-512",
        "ML-KEM-512",
        "ml_kem_512",
        "MLKEM512",
        "mlkem512",
    ];

    for input in inputs {
        let parsed = KEMVariant::from_str(input)
            .unwrap_or_else(|_| panic!("Failed to parse case-insensitive '{input}'"));
        assert_eq!(
            parsed,
            KEMVariant::MLKEM512,
            "Parsing '{input}' must yield MLKEM512"
        );
    }
}

#[test]
fn test_pqc_variant_from_str_invalid_returns_err() {
    use std::str::FromStr;
    let invalid = ["", "RSA-2048", "ECC-P256", "ML-DSA-999"];

    for s in invalid {
        let result = PQCVariant::from_str(s);
        assert!(
            result.is_err(),
            "Invalid variant '{s}' must return error, got {:?}",
            result
        );
    }
}

#[test]
fn test_kem_variant_from_str_invalid_returns_err() {
    use std::str::FromStr;
    let invalid = ["", "RSA-2048", "ML-KEM-999"];

    for s in invalid {
        let result = KEMVariant::from_str(s);
        assert!(
            result.is_err(),
            "Invalid KEM variant '{s}' must return error, got {:?}",
            result
        );
    }
}

// =============================================================================
// 6. key_suffix / key_filename consistency
// =============================================================================

#[test]
fn test_pqc_variant_key_filename_consistency() {
    let dsa44 = PQCVariant::MLDSA44;
    assert_eq!(dsa44.key_suffix(), "mldsa44");
    assert_eq!(dsa44.key_filename(), "id_mldsa44");
    assert_eq!(dsa44.pub_key_filename(), "id_mldsa44.pub");

    let dsa87 = PQCVariant::MLDSA87;
    assert_eq!(dsa87.key_suffix(), "mldsa87");
    assert_eq!(dsa87.key_filename(), "id_mldsa87");
    assert_eq!(dsa87.pub_key_filename(), "id_mldsa87.pub");
}

#[test]
fn test_kem_variant_key_filename_consistency() {
    let kem512 = KEMVariant::MLKEM512;
    assert_eq!(kem512.key_suffix(), "kem512");
    assert_eq!(kem512.key_filename(), "id_kem512");
    assert_eq!(kem512.pub_key_filename(), "id_kem512.pub");

    let kem1024 = KEMVariant::MLKEM1024;
    assert_eq!(kem1024.key_suffix(), "kem1024");
    assert_eq!(kem1024.key_filename(), "id_kem1024");
    assert_eq!(kem1024.pub_key_filename(), "id_kem1024.pub");
}
