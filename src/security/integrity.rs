use crate::consts::{CONFIG_FILE_PATH, LLM_CLI_BASE_DIR};
use crate::security::identity::IdentityManager;
use crate::security::pqc::{MldsaVariant, PqcProvider};
use anyhow::{anyhow, Result};
use base64::{engine::general_purpose, Engine as _};
use rsa::pkcs1v15::Pkcs1v15Sign;
use rsa::{pkcs8::DecodePrivateKey, pkcs8::DecodePublicKey, RsaPrivateKey, RsaPublicKey};
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
    pub rsa_signature: Option<String>,
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
    /// Calculates a combined integrity hash of all tool binaries.
    /// This uses SHA-256 as the underlying primitive, which is then signed by ML-DSA.
    /// In a post-quantum context, this hash acts as the message digest for the ML-DSA signature.
    fn calculate_binary_hash(&self) -> Result<String> {
        let exe_path = std::env::current_exe()?;
        let bin_dir = exe_path
            .parent()
            .ok_or_else(|| anyhow!("Could not get binary directory"))?;

        let tool_binaries = ["llsc"];

        let mut found_any = false;
        let mut overall_hasher = Sha256::new();

        let mut sorted_tools = tool_binaries.to_vec();
        sorted_tools.sort();

        for bin_name in sorted_tools {
            let bin_path = bin_dir.join(bin_name);
            if bin_path.exists() && bin_path.is_file() {
                let bytes = fs::read(&bin_path)?;
                let mut file_hasher = Sha256::new();
                file_hasher.update(bytes);
                // Combine file hashes into the overall digest
                overall_hasher.update(file_hasher.finalize());
                found_any = true;
            }
        }

        if !found_any {
            let bytes = fs::read(&exe_path)?;
            let mut hasher = Sha256::new();
            hasher.update(bytes);
            return Ok(hex::encode(hasher.finalize()));
        }

        Ok(hex::encode(overall_hasher.finalize()))
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

        let mut data_to_sign = BTreeMap::new();
        data_to_sign.insert("binary_hash", &binary_hash);
        data_to_sign.insert("config_hash", &config_hash);
        let json_data = serde_json::to_string(&data_to_sign)?;

        // 1. Sign with PQC (ML-DSA)
        let sk_pqc = IdentityManager::get_pqc_private_key(MldsaVariant::Mldsa65)
            .map_err(|_| anyhow!("Identity keys not found. Run 'keygen' first."))?;
        let pqc_sig =
            PqcProvider::sign_mldsa(json_data.as_bytes(), &sk_pqc, MldsaVariant::Mldsa65)?;

        // 2. Sign with RSA (Classical)
        let rsa_priv_pem = IdentityManager::get_rsa_private_key_pem()?;
        let rsa_priv = RsaPrivateKey::from_pkcs8_pem(&rsa_priv_pem)
            .map_err(|e| anyhow!("Failed to load RSA private key: {}", e))?;
        let rsa_digest = Sha256::digest(json_data.as_bytes());
        let rsa_sig = rsa_priv
            .sign(Pkcs1v15Sign::new::<Sha256>(), &rsa_digest)
            .map_err(|e| anyhow!("RSA signing failed: {}", e))?;

        let manifest = IntegrityManifest {
            binary_hash,
            config_hash,
            pqc_signature: general_purpose::STANDARD.encode(pqc_sig),
            pqc_algorithm: "ML-DSA-65".to_string(),
            rsa_signature: Some(general_purpose::STANDARD.encode(rsa_sig)),
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
        log::debug!(
            "IntegrityVerifier: Starting system integrity verification using manifest at {:?}",
            self.manifest_path
        );
        if !self.manifest_path.exists() {
            return Err(anyhow!(
                "Integrity manifest not found. Run 'manifest' to establish a baseline."
            ));
        }

        let content = fs::read_to_string(&self.manifest_path)?;
        let manifest: IntegrityManifest = serde_json::from_str(&content)?;

        // 1. Verify PQC Signature of the manifest itself
        let mut data_to_verify = BTreeMap::new();
        data_to_verify.insert("binary_hash", &manifest.binary_hash);
        data_to_verify.insert("config_hash", &manifest.config_hash);
        let json_data = serde_json::to_string(&data_to_verify)?;

        log::debug!(
            "IntegrityVerifier: Verifying PQC signature (algo: {})",
            manifest.pqc_algorithm
        );
        let pk_pqc = IdentityManager::get_pqc_public_key(MldsaVariant::Mldsa65)?;
        let signature_pqc = general_purpose::STANDARD.decode(&manifest.pqc_signature)?;

        if !PqcProvider::verify_mldsa(
            json_data.as_bytes(),
            &signature_pqc,
            &pk_pqc,
            MldsaVariant::Mldsa65,
        ) {
            log::warn!("IntegrityVerifier: PQC signature verification FAILED");
            return Ok(false); // PQC signature mismatch
        }

        // 1b. Verify RSA Signature (Classical)
        if let Some(rsa_sig_b64) = &manifest.rsa_signature {
            log::debug!("IntegrityVerifier: Verifying classical RSA signature");
            let rsa_pub_path = crate::consts::KEY_DIR.join("id_rsa.pub");
            let rsa_pub_pem = fs::read_to_string(rsa_pub_path)?;
            let rsa_pub = RsaPublicKey::from_public_key_pem(&rsa_pub_pem)
                .map_err(|e| anyhow!("Failed to load RSA public key: {}", e))?;
            let rsa_sig = general_purpose::STANDARD.decode(rsa_sig_b64)?;
            let rsa_digest = Sha256::digest(json_data.as_bytes());

            if rsa_pub
                .verify(Pkcs1v15Sign::new::<Sha256>(), &rsa_digest, &rsa_sig)
                .is_err()
            {
                log::warn!("IntegrityVerifier: RSA signature verification FAILED");
                return Ok(false); // RSA signature mismatch
            }
        }

        // 2. Compare current file hashes with the manifest
        let current_binary = self.calculate_binary_hash()?;
        log::debug!(
            "IntegrityVerifier: Binary hash check: expected={}, actual={}",
            manifest.binary_hash,
            current_binary
        );
        if current_binary != manifest.binary_hash {
            log::warn!("IntegrityVerifier: Binary hash mismatch");
            return Ok(false); // Binary has been modified
        }

        let current_config = self.calculate_config_hash()?;
        log::debug!(
            "IntegrityVerifier: Config hash check: expected={}, actual={}",
            manifest.config_hash,
            current_config
        );
        if current_config != manifest.config_hash {
            log::warn!("IntegrityVerifier: Config hash mismatch");
            return Ok(false); // Configuration has been modified
        }

        log::debug!("IntegrityVerifier: System integrity verified successfully");
        Ok(true)
    }
}
