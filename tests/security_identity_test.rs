#![allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use llm_secure_cli::security::identity::IdentityManager;
    use llm_secure_cli::security::pqc::{PQCVariant, PqcProvider};
    use std::sync::OnceLock;

    /// Set up a temporary base directory for identity tests so that
    /// test keys are never written to ~/.llsc.
    fn setup_temp_basedir() {
        static INIT: OnceLock<()> = OnceLock::new();
        INIT.get_or_init(|| {
            let dir = tempfile::tempdir().expect("Failed to create temp dir");
            let path = dir.path().to_path_buf();
            // Keep the TempDir alive for the duration of all tests (leak it intentionally)
            std::mem::forget(dir);
            llm_secure_cli::consts::init_base_dir(Some(path));
        });
    }

    #[test]
    fn test_pqc_variants_sign_verify() {
        let message = b"Critical security operation: format drive";

        {
            let variant = PQCVariant::MLDSA87;
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
    fn test_identity_key_generation() {
        // Use a temporary directory so tests don't touch ~/.llsc
        setup_temp_basedir();

        // Generate unencrypted PQC + KEM keys
        IdentityManager::ensure_keys_with_passphrase(None).expect("Failed to ensure keys");

        // Verify keys exist
        assert!(IdentityManager::has_keys(), "Identity keys should exist");

        // Verify we can read the public keys
        let pqc_pub = IdentityManager::get_pqc_public_key(PQCVariant::MLDSA44)
            .expect("Failed to read PQC public key");
        assert!(!pqc_pub.is_empty(), "PQC public key should not be empty");

        let kem_pub = IdentityManager::get_kem_public_key().expect("Failed to read KEM public key");
        assert!(!kem_pub.is_empty(), "KEM public key should not be empty");
    }
}
