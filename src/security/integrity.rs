use crate::consts::{CONFIG_FILE_PATH, LLM_CLI_BASE_DIR};
use crate::security::identity::IdentityManager;
use crate::security::pqc::{MldsaVariant, PqcProvider};
use anyhow::{anyhow, Result};
use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Debug)]
pub struct IntegrityManifest {
    pub binary_hash: String,
    pub config_hash: String,
    pub pqc_signature: String,
    pub pqc_algorithm: String,
}

pub struct IntegrityVerifier {
    pub manifest_path: PathBuf,
}

impl IntegrityVerifier {
    pub fn new() -> Self {
        let mut manifest_path = LLM_CLI_BASE_DIR.clone();
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
    /// Calculates the SHA-256 hash of the current executable binary.
    fn calculate_binary_hash(&self) -> Result<String> {
        let exe_path = std::env::current_exe()?;
        let bytes = fs::read(exe_path)?;
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        Ok(hex::encode(hasher.finalize()))
    }

    /// Calculates the SHA-256 hash of the configuration file.
    fn calculate_config_hash(&self) -> Result<String> {
        if !CONFIG_FILE_PATH.exists() {
            return Ok("MISSING".to_string());
        }
        let bytes = fs::read(&*CONFIG_FILE_PATH)?;
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        Ok(hex::encode(hasher.finalize()))
    }

    /// Generates a new integrity manifest, signs it with PQC, and saves it to disk.
    pub fn rebuild_manifest(&self) -> Result<()> {
        let binary_hash = self.calculate_binary_hash()?;
        let config_hash = self.calculate_config_hash()?;

        let mut data_to_sign = HashMap::new();
        data_to_sign.insert("binary_hash", &binary_hash);
        data_to_sign.insert("config_hash", &config_hash);
        let json_data = serde_json::to_string(&data_to_sign)?;

        // Sign the manifest data with the PQC private key
        let sk = IdentityManager::get_pqc_private_key(MldsaVariant::Mldsa65)
            .map_err(|_| anyhow!("Identity keys not found. Run 'keygen' first."))?;

        let signature = PqcProvider::sign_mldsa(json_data.as_bytes(), &sk, MldsaVariant::Mldsa65)?;

        let manifest = IntegrityManifest {
            binary_hash,
            config_hash,
            pqc_signature: general_purpose::STANDARD.encode(signature),
            pqc_algorithm: "ML-DSA-65".to_string(),
        };

        let json_manifest = serde_json::to_string_pretty(&manifest)?;
        if let Some(parent) = self.manifest_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.manifest_path, json_manifest)?;

        Ok(())
    }

    /// Verifies the current system state against the saved manifest.
    /// Returns true if integrity is confirmed, false if tampering is detected.
    pub fn verify(&self) -> Result<bool> {
        if !self.manifest_path.exists() {
            return Err(anyhow!(
                "Integrity manifest not found. Run 'manifest' to establish a baseline."
            ));
        }

        let content = fs::read_to_string(&self.manifest_path)?;
        let manifest: IntegrityManifest = serde_json::from_str(&content)?;

        // 1. Verify PQC Signature of the manifest itself
        let mut data_to_verify = HashMap::new();
        data_to_verify.insert("binary_hash", &manifest.binary_hash);
        data_to_verify.insert("config_hash", &manifest.config_hash);
        let json_data = serde_json::to_string(&data_to_verify)?;

        let pk = IdentityManager::get_pqc_public_key(MldsaVariant::Mldsa65)?;
        let signature = general_purpose::STANDARD.decode(&manifest.pqc_signature)?;

        if !PqcProvider::verify_mldsa(json_data.as_bytes(), &signature, &pk, MldsaVariant::Mldsa65)
        {
            return Ok(false); // Manifest signature mismatch (Manifest itself tampered)
        }

        // 2. Compare current file hashes with the manifest
        let current_binary = self.calculate_binary_hash()?;
        if current_binary != manifest.binary_hash {
            return Ok(false); // Binary has been modified
        }

        let current_config = self.calculate_config_hash()?;
        if current_config != manifest.config_hash {
            return Ok(false); // Configuration has been modified
        }

        Ok(true)
    }
}
