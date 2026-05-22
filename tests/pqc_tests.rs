use llm_secure_cli::security::pqc::{
    MldsaVariant, PQCVariant, PqcProvider, ResponseSigner, SecureStorage,
};

#[test]
fn test_mldsa_sign_verify_all_variants() {
    let variants = [
        MldsaVariant::MLDSA44,
        MldsaVariant::MLDSA65,
        MldsaVariant::MLDSA87,
    ];
    let message = b"Post-quantum security is essential for 2026.";

    for variant in variants {
        let (pk, sk) = PqcProvider::generate_keypair(variant).expect("Key generation failed");
        let sig = PqcProvider::sign_mldsa(message, &sk, variant).expect("Signing failed");

        assert!(
            PqcProvider::verify_mldsa(message, &sig, &pk, variant),
            "Verification failed for variant {:?}",
            variant
        );

        // Negative test: wrong message
        let tampered_msg = b"Tampered message";
        assert!(
            !PqcProvider::verify_mldsa(tampered_msg, &sig, &pk, variant),
            "Verification should fail for tampered message (variant {:?})",
            variant
        );

        // Negative test: wrong signature
        let mut tampered_sig = sig.clone();
        if let Some(byte) = tampered_sig.get_mut(0) {
            *byte ^= 0xFF;
        }
        assert!(
            !PqcProvider::verify_mldsa(message, &tampered_sig, &pk, variant),
            "Verification should fail for tampered signature"
        );
    }
}

#[test]
fn test_mlkem_encaps_decaps() {
    // We use the provider's specific ML-KEM-768 implementation
    let v = saorsa_pqc::api::MlKemVariant::MlKem768;

    // We need the secret key as bytes for the provider
    let (pk_gen, sk_gen) = saorsa_pqc::api::MlKem::new(v)
        .generate_keypair()
        .expect("keygen failed");
    let pk_bytes = pk_gen.to_bytes();
    let sk_bytes = sk_gen.to_bytes();

    let (ss_enc, ct) = PqcProvider::encapsulate(
        llm_secure_cli::security::pqc::KEMVariant::MLKEM768,
        &pk_bytes,
    )
    .expect("Encapsulation failed");
    let ss_dec = PqcProvider::decapsulate(
        llm_secure_cli::security::pqc::KEMVariant::MLKEM768,
        &ct,
        &sk_bytes,
    )
    .expect("Decapsulation failed");

    assert_eq!(ss_enc, ss_dec, "Shared secret mismatch");
    assert!(
        !ss_enc.iter().all(|&b| b == 0),
        "Shared secret should not be all zeros"
    );
}

#[test]
fn test_secure_storage_hybrid_encryption() {
    let v = saorsa_pqc::api::MlKemVariant::MlKem768;
    let (pk, sk) = saorsa_pqc::api::MlKem::new(v)
        .generate_keypair()
        .expect("KEM keygen failed");
    let original_data = b"Sensitive post-quantum data content";

    let packet = SecureStorage::encrypt(original_data, &pk.to_bytes()).expect("Encryption failed");
    let decrypted_data =
        SecureStorage::decrypt(&packet, &sk.to_bytes()).expect("Decryption failed");

    assert_eq!(
        original_data.to_vec(),
        decrypted_data,
        "Decrypted data does not match original"
    );
    assert_eq!(packet.algo, "ML-KEM-768/AES-256-GCM");
}

#[test]
fn test_response_signer() {
    let variant = MldsaVariant::MLDSA65;
    let (pk, sk) = PqcProvider::generate_keypair(variant).expect("Keygen failed");
    let response_text = "The quick brown fox jumps over the lazy dog";
    let verification_id = "test-v-id-123";

    let signed = ResponseSigner::sign_response(response_text, verification_id, &sk, variant)
        .expect("Response signing failed");

    assert_eq!(signed["result"], response_text);
    assert_eq!(signed["verification_id"], verification_id);
    assert_eq!(signed["algorithm"], variant.to_str());

    // Manual verification since verify_response was missing
    let msg = format!("{}:{}", verification_id, response_text);
    let sig_b64 = signed["pqc_signature"]
        .as_str()
        .expect("pqc_signature should be a string");
    let sig = base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, sig_b64)
        .expect("Base64 decode should succeed");

    assert!(
        PqcProvider::verify(variant, &pk, msg.as_bytes(), &sig).is_ok(),
        "Response verification failed"
    );
}

#[test]
fn test_merkle_session_verification_logic() {
    use llm_secure_cli::security::audit::AuditEntry;
    use llm_secure_cli::security::merkle::MerkleTree;
    use sha2::{Digest, Sha256};

    let mut entry = AuditEntry {
        timestamp: "2026-04-22T10:00:00Z".to_string(),
        trace_id: "test-session".to_string(),
        subject: "user".to_string(),
        audience: "-".to_string(),
        model: "test-model".to_string(),
        provider: "test-provider".to_string(),
        event_type: "tool_call".to_string(),
        tool: "test_tool".to_string(),
        args: serde_json::json!({"cmd": "ls"}),
        pqc_confidential: false,
        output: None,
        status: llm_secure_cli::security::audit::AuditStatus::Success,
        exit_code: Some(0),
        prev_hash: "0".repeat(64),
        hash: String::new(),
        pqc_signature: None,
        pqc_algorithm: None,
        hostname: "test-host".to_string(),
        os: "linux".to_string(),
        arch: "x86_64".to_string(),
        cli_version: "0.1.0".to_string(),
    };

    let entry_json = serde_json::to_string(&entry).expect("entry serialization should succeed");
    let mut hasher = Sha256::new();
    hasher.update(entry_json.as_bytes());
    entry.hash = llm_secure_cli::utils::hex_encode(hasher.finalize());

    let leaf_hashes = vec![entry.hash.clone()];
    let tree = MerkleTree::new(leaf_hashes.clone());
    let root = tree.root_hex.clone();

    let mut entry_to_verify: AuditEntry = serde_json::from_value(
        serde_json::to_value(&entry).expect("entry to_value should succeed"),
    )
    .expect("entry from_value should succeed");
    entry_to_verify.hash = String::new();
    entry_to_verify.pqc_signature = None;
    entry_to_verify.pqc_algorithm = None;

    let verify_json = serde_json::to_string(&entry_to_verify)
        .expect("entry_to_verify serialization should succeed");
    let mut verify_hasher = Sha256::new();
    verify_hasher.update(verify_json.as_bytes());
    let recalculated_hash = llm_secure_cli::utils::hex_encode(verify_hasher.finalize());

    assert_eq!(
        entry.hash, recalculated_hash,
        "Recalculated hash must match original hash"
    );

    let verify_tree = MerkleTree::new(vec![recalculated_hash]);
    assert_eq!(root, verify_tree.root_hex, "Merkle root mismatch");
}

#[test]
fn test_pqc_agility_manager() {
    use llm_secure_cli::config::ConfigManager;
    use llm_secure_cli::security::pqc::PQCAgilityManager;

    let config_manager = ConfigManager::new();

    {
        let mut config = (*config_manager.get_config().expect("Failed to get config")).clone();
        config.security.security_level = llm_secure_cli::config::models::SecurityLevel::Standard;
        config.security.high_risk_tools = vec!["execute_python".to_string()];
        config.security.scaling_patterns = vec!["/etc/shadow".to_string()];
        config.security.dual_llm_verification = Some(true);
        let _ = config_manager.set_config(config);
    }

    let config = config_manager.get_config().expect("Failed to get config");

    // Normal tool, low risk
    let level = PQCAgilityManager::get_required_level(&config, "ls", None);
    assert!(matches!(level, PQCVariant::MLDSA44));

    // High risk tool
    let level = PQCAgilityManager::get_required_level(&config, "execute_python", None);
    assert_eq!(level, PQCVariant::MLDSA87);

    // Sensitive context (Escalates to High)
    let args = serde_json::json!({"path": "/etc/shadow"});
    let level = PQCAgilityManager::get_required_level(&config, "read_file", Some(&args));
    assert_eq!(level, PQCVariant::MLDSA87);
}

#[test]
fn test_hybrid_cose_signer() {
    use ed25519_dalek::SigningKey;
    use llm_secure_cli::security::pqc_cose::HybridSigner;
    use rand::rngs::OsRng;

    let variant = MldsaVariant::MLDSA65;

    // Generate keys in memory for the test to avoid filesystem side effects
    let mut rng = OsRng;
    let classical_signing_key = SigningKey::generate(&mut rng);
    let classical_priv = classical_signing_key.to_bytes().to_vec();
    let classical_pub = classical_signing_key.verifying_key().to_bytes().to_vec();

    let (pqc_pub, pqc_priv) =
        PqcProvider::generate_keypair(variant).expect("PQC keypair generation should succeed");

    let payload_val = serde_json::json!({"msg": "Hybrid security token", "exp": 1776824283});
    let mut payload = Vec::new();
    ciborium::into_writer(&payload_val, &mut payload).expect("CBOR encoding should succeed");

    let token = HybridSigner::create_hybrid_token(&payload, &classical_priv, &pqc_priv, variant)
        .expect("Failed to create token");
    assert!(!token.is_empty());

    let verified_payload =
        HybridSigner::verify_hybrid_token(&token, &classical_pub, |_| pqc_pub.clone());

    assert!(verified_payload.is_some());
    assert_eq!(
        verified_payload.expect("verified_payload should be Some")["msg"],
        "Hybrid security token"
    );
}
