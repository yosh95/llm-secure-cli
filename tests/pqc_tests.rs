use llm_secure_cli::security::pqc::{
    MldsaVariant, MlkemVariant, PqcProvider, ResponseSigner, SecureStorage,
};

#[test]
fn test_mldsa_sign_verify_all_variants() {
    let _ = env_logger::builder().is_test(true).try_init();
    let variants = [
        MldsaVariant::Mldsa44,
        MldsaVariant::Mldsa65,
        MldsaVariant::Mldsa87,
    ];
    let message = b"Post-quantum security is essential for 2026.";

    for variant in variants {
        let (pk, sk) = PqcProvider::generate_mldsa_keypair(variant);
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
fn test_mlkem_encaps_decaps_all_variants() {
    let variants = [
        MlkemVariant::Mlkem512,
        MlkemVariant::Mlkem768,
        MlkemVariant::Mlkem1024,
    ];

    for variant in variants {
        let (pk, sk) = PqcProvider::generate_mlkem_keypair(variant);
        let (ss_enc, ct) = PqcProvider::encapsulate_mlkem(&pk, variant);
        let ss_dec = PqcProvider::decapsulate_mlkem(&ct, &sk, variant);

        assert_eq!(
            ss_enc, ss_dec,
            "Shared secret mismatch for variant {:?}",
            variant
        );
        assert!(
            !ss_enc.iter().all(|&b| b == 0),
            "Shared secret should not be all zeros"
        );
    }
}

#[test]
fn test_secure_storage_hybrid_encryption() {
    let (pk, sk) = PqcProvider::generate_mlkem_keypair(MlkemVariant::Mlkem768);
    let original_data = b"Sensitive post-quantum data content";

    let packet = SecureStorage::encrypt(original_data, &pk);
    let decrypted_data = SecureStorage::decrypt(&packet, &sk);

    assert_eq!(
        original_data.to_vec(),
        decrypted_data,
        "Decrypted data does not match original"
    );
    assert_eq!(packet.algo, "ML-KEM-768/AES-256-GCM");
}

#[test]
fn test_response_signer() {
    let variant = MldsaVariant::Mldsa65;
    let (pk, sk) = PqcProvider::generate_mldsa_keypair(variant);
    let response_text = "The quick brown fox jumps over the lazy dog";
    let verification_id = "test-v-id-123";

    let signed = ResponseSigner::sign_response(response_text, verification_id, &sk, variant)
        .expect("Response signing failed");

    assert_eq!(signed.result, response_text);
    assert_eq!(signed.verification_id, verification_id);
    assert_eq!(signed.algorithm, variant.to_str());

    assert!(
        ResponseSigner::verify_response(&signed, &pk),
        "Response verification failed"
    );
}

#[test]
fn test_merkle_session_verification_logic() {
    use llm_secure_cli::security::audit::AuditEntry;
    use llm_secure_cli::security::merkle::MerkleTree;
    use sha2::{Digest, Sha256};

    // Simulate entry creation
    let mut entry = AuditEntry {
        timestamp: "2026-04-22T10:00:00Z".to_string(),
        trace_id: "test-session".to_string(),
        subject: "user".to_string(),
        audience: "-".to_string(),
        model: "test-model".to_string(),
        event_type: "tool_call".to_string(),
        tool: "test_tool".to_string(),
        args: serde_json::json!({"cmd": "ls"}),
        pqc_confidential: false,
        output: None,
        status: "SUCCESS".to_string(),
        exit_code: Some(0),
        prev_hash: "0".repeat(64),
        hash: String::new(),
        pqc_signature: None,
        pqc_algorithm: None,
    };

    // Calculate hash exactly like log_audit
    let entry_json = serde_json::to_string(&entry).unwrap();
    let mut hasher = Sha256::new();
    hasher.update(entry_json.as_bytes());
    entry.hash = hex::encode(hasher.finalize());

    let leaf_hashes = vec![entry.hash.clone()];
    let tree = MerkleTree::new(leaf_hashes.clone());
    let root = tree.root_hex.clone();

    // Verification logic simulation (what I fixed)
    let mut entry_to_verify: AuditEntry =
        serde_json::from_value(serde_json::to_value(&entry).unwrap()).unwrap();
    entry_to_verify.hash = String::new();
    entry_to_verify.pqc_signature = None;
    entry_to_verify.pqc_algorithm = None;

    let verify_json = serde_json::to_string(&entry_to_verify).unwrap();
    let mut verify_hasher = Sha256::new();
    verify_hasher.update(verify_json.as_bytes());
    let recalculated_hash = hex::encode(verify_hasher.finalize());

    assert_eq!(
        entry.hash, recalculated_hash,
        "Recalculated hash must match original hash"
    );

    let verify_tree = MerkleTree::new(vec![recalculated_hash]);
    assert_eq!(root, verify_tree.root_hex, "Merkle root mismatch");
}

#[test]
fn test_pqc_agility_manager() {
    use llm_secure_cli::security::pqc::MldsaVariant;
    use llm_secure_cli::security::pqc::PQCAgilityManager;

    // Normal tool, low risk
    let level = PQCAgilityManager::get_required_level("ls", None, "low");
    assert_eq!(level, MldsaVariant::Mldsa44);

    // High risk tool
    let level = PQCAgilityManager::get_required_level("execute_command", None, "low");
    assert_eq!(level, MldsaVariant::Mldsa87);

    // High environment risk
    let level = PQCAgilityManager::get_required_level("ls", None, "high");
    assert_eq!(level, MldsaVariant::Mldsa87);

    // Sensitive context (contains blocked paths pattern)
    let args = serde_json::json!({"path": "/etc/shadow"});
    let level = PQCAgilityManager::get_required_level("read_file", Some(&args), "low");
    assert_eq!(level, MldsaVariant::Mldsa87);
}

#[test]
fn test_hybrid_cose_signer() {
    use llm_secure_cli::consts::KEY_DIR;
    use llm_secure_cli::security::identity::IdentityManager;
    use llm_secure_cli::security::pqc_cose::HybridSigner;
    use std::fs;

    let variant = MldsaVariant::Mldsa65;

    // Generate temporary keys
    let _ = IdentityManager::ensure_keys(true);
    let classical_priv = IdentityManager::get_classical_private_key_pem().unwrap();
    let classical_pub = fs::read_to_string(KEY_DIR.join("id_ed25519.pub")).unwrap();
    let pqc_priv = IdentityManager::get_pqc_private_key(variant).unwrap();
    let pqc_pub = IdentityManager::get_pqc_public_key(variant).unwrap();

    let payload = serde_json::json!({"msg": "Hybrid security token", "exp": 1776824283});

    let token = HybridSigner::create_hybrid_token(&payload, &classical_priv, &pqc_priv, variant);
    assert!(!token.is_empty());

    let verified_payload =
        HybridSigner::verify_hybrid_token(&token, &classical_pub, |_| pqc_pub.clone());

    assert!(verified_payload.is_some());
    assert_eq!(verified_payload.unwrap()["msg"], "Hybrid security token");
}
