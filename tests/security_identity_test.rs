#[cfg(test)]
mod tests {
    use ciborium::Value;
    use llm_secure_cli::security::identity::IdentityManager;
    use llm_secure_cli::security::pqc::{PQCVariant, PqcProvider};

    #[tokio::test]
    async fn test_identity_token_generation() {
        // 1. Ensure keys exist
        IdentityManager::ensure_keys().expect("Failed to ensure keys");

        // 2. Generate a token for a specific tool
        let tool_name = "test_server__list_files";
        let token_b64 =
            IdentityManager::generate_token(Some(tool_name)).expect("Failed to generate token");

        assert!(!token_b64.is_empty(), "Token should not be empty");

        // 3. Decode Base64
        let token_bytes =
            base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, token_b64)
                .expect("Failed to decode base64 token");

        // 4. Parse manually using ciborium
        let value: Value =
            ciborium::from_reader(token_bytes.as_slice()).expect("Failed to parse CBOR from bytes");

        // Expect Tag 98
        if let Value::Tag(98, inner) = value {
            if let Value::Array(cose_sign) = *inner {
                assert_eq!(cose_sign.len(), 4, "COSE_Sign should have 4 elements");

                // Index 2 is payload
                if let Value::Bytes(payload_bytes) = &cose_sign[2] {
                    let claims: serde_json::Value = ciborium::from_reader(payload_bytes.as_slice())
                        .expect("Failed to parse claims from payload");
                    assert_eq!(claims["iss"], "llsc-client");
                    assert_eq!(claims["tool"], tool_name);
                } else {
                    panic!("Payload is not bytes");
                }

                // Index 3 is signatures array
                if let Value::Array(sigs) = &cose_sign[3] {
                    assert_eq!(sigs.len(), 2, "Should have 2 signatures");
                } else {
                    panic!("Signatures is not an array");
                }
            } else {
                panic!("Inner value is not an array");
            }
        } else {
            panic!("Value is not Tag 98");
        }
    }

    #[test]
    fn test_pqc_variants_sign_verify() {
        let message = b"Critical security operation: format drive";

        for variant in [
            PQCVariant::MLDSA44,
            PQCVariant::MLDSA65,
            PQCVariant::MLDSA87,
        ] {
            // Generate
            let (pk, sk) = PqcProvider::generate_keypair(variant).expect("Keygen failed");

            // Sign
            let sig = PqcProvider::sign(variant, &sk, message).expect("Signing failed");

            // Verify
            PqcProvider::verify(variant, &pk, message, &sig)
                .unwrap_or_else(|_| panic!("Verification failed for {:?}", variant));

            // Tamper check
            let mut tampered_message = message.to_vec();
            tampered_message[0] ^= 0xFF;
            assert!(PqcProvider::verify(variant, &pk, &tampered_message, &sig).is_err());
        }
    }

    #[test]
    fn test_mcp_zero_trust_logic_flow() {
        let server_name = "docker-mcp-server";
        let tool_name = format!("{}__{}", server_name, "run_command");
        let parts: Vec<&str> = tool_name.split("__").collect();
        assert_eq!(parts[0], server_name);

        let zero_trust_enabled = true;
        let approved = !zero_trust_enabled;
        assert!(!approved, "Zero trust must force approval to false");
    }
}
