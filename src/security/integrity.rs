use crate::consts::{config_file_path, get_base_dir};
use crate::security::identity::IdentityManager;
use crate::security::pqc::{MldsaVariant, PqcProvider};
use anyhow::{Result, anyhow};
use base64::{Engine as _, engine::general_purpose};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use pkcs8::DecodePrivateKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Debug)]
pub struct IntegrityManifest {
    pub binary_hash: String,
    pub config_hash: String,
    pub pqc_signature: String,
    pub pqc_algorithm: String,
    pub classical_signature: Option<String>,
}

pub struct IntegrityVerifier {
    pub manifest_path: PathBuf,
}

impl IntegrityVerifier {
    pub fn new() -> Self {
        let mut manifest_path = get_base_dir().clone();
        manifest_path.push("integrity_manifest.json");
        Self { manifest_path }
    }
}

impl Default for IntegrityVerifier {
    fn default() -> Self {
        Self::new()
    }
}

impl IntegrityVerifier {
    /// Calculates a combined integrity hash of all tool binaries.
    fn calculate_binary_hash(&self) -> Result<String> {
        let exe_path = std::env::current_exe()?;
        let mut file = fs::File::open(&exe_path)?;
        let mut hasher = Sha256::new();

        let mut buffer = [0u8; 8192];
        use std::io::Read;
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
        Ok(crate::utils::hex_encode(hasher.finalize()))
    }

    /// Calculates the SHA-256 hash of the configuration file.
    fn calculate_config_hash(&self) -> Result<String> {
        let c_path = config_file_path();
        if !c_path.exists() {
            return Ok("MISSING".to_string());
        }
        let mut file = fs::File::open(&c_path)?;
        let mut hasher = Sha256::new();

        let mut buffer = [0u8; 8192];
        use std::io::Read;
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
        Ok(crate::utils::hex_encode(hasher.finalize()))
    }

    /// Generates a new integrity manifest, signs it with PQC, and saves it to disk.
    pub fn rebuild_manifest(&self) -> Result<()> {
        let binary_hash = self.calculate_binary_hash()?;
        let config_hash = self.calculate_config_hash()?;

        let mut data_to_sign = BTreeMap::new();
        data_to_sign.insert("binary_hash", &binary_hash);
        data_to_sign.insert("config_hash", &config_hash);
        let json_data = serde_json::to_string(&data_to_sign)?;

        // 1. Sign with PQC (ML-DSA)
        let sk_pqc = IdentityManager::get_pqc_private_key(MldsaVariant::MLDSA65)
            .map_err(|_| anyhow!("Identity keys not found. Run 'keygen' first."))?;
        let pqc_sig =
            PqcProvider::sign_mldsa(json_data.as_bytes(), &sk_pqc, MldsaVariant::MLDSA65)?;

        // 2. Sign with Ed25519 (Classical)
        let classical_priv_pem = IdentityManager::get_classical_private_key_pem()?;
        let signing_key = SigningKey::from_pkcs8_pem(&classical_priv_pem)
            .map_err(|e| anyhow!("Failed to load Ed25519 private key: {}", e))?;
        let classical_sig = signing_key.sign(json_data.as_bytes());

        let manifest = IntegrityManifest {
            binary_hash,
            config_hash,
            pqc_signature: general_purpose::STANDARD.encode(pqc_sig),
            pqc_algorithm: "ML-DSA-65".to_string(),
            classical_signature: Some(general_purpose::STANDARD.encode(classical_sig.to_vec())),
        };

        let json_manifest = serde_json::to_string_pretty(&manifest)?;
        if let Some(parent) = self.manifest_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.manifest_path, json_manifest)?;

        Ok(())
    }

    /// Verifies the current system state against the saved manifest.
    pub fn verify(&self) -> Result<bool> {
        if !self.manifest_path.exists() {
            return Err(anyhow!(
                "Integrity manifest not found. Run 'manifest' to establish a baseline."
            ));
        }

        let content = fs::read_to_string(&self.manifest_path)?;
        let manifest: IntegrityManifest = serde_json::from_str(&content)?;

        // 1. Verify PQC Signature of the manifest
        let mut data_to_verify = BTreeMap::new();
        data_to_verify.insert("binary_hash", &manifest.binary_hash);
        data_to_verify.insert("config_hash", &manifest.config_hash);
        let json_data = serde_json::to_string(&data_to_verify)?;

        let pk_pqc = IdentityManager::get_pqc_public_key(MldsaVariant::MLDSA65)?;
        let signature_pqc = general_purpose::STANDARD.decode(&manifest.pqc_signature)?;

        if !PqcProvider::verify_mldsa(
            json_data.as_bytes(),
            &signature_pqc,
            &pk_pqc,
            MldsaVariant::MLDSA65,
        ) {
            return Ok(false);
        }

        // 2. Verify Classical Signature (Ed25519)
        if let Some(classical_sig_b64) = &manifest.classical_signature {
            let classical_pub_bytes = IdentityManager::get_classical_public_key()?;
            let verifying_key = VerifyingKey::from_bytes(
                &classical_pub_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid Ed25519 public key length"))?,
            )
            .map_err(|e| anyhow!("Failed to load Ed25519 public key: {}", e))?;
            let classical_sig_bytes = general_purpose::STANDARD.decode(classical_sig_b64)?;
            let classical_sig = Signature::from_slice(&classical_sig_bytes)
                .map_err(|e| anyhow!("Invalid Ed25519 signature format: {}", e))?;

            if verifying_key
                .verify(json_data.as_bytes(), &classical_sig)
                .is_err()
            {
                return Ok(false);
            }
        }

        // 3. Compare current file hashes with the manifest
        let current_binary = self.calculate_binary_hash()?;
        if current_binary != manifest.binary_hash {
            return Ok(false);
        }

        let current_config = self.calculate_config_hash()?;
        if current_config != manifest.config_hash {
            return Ok(false);
        }

        Ok(true)
    }
}
