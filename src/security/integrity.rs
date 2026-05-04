use crate::consts::{CONFIG_FILE_PATH, LLM_CLI_BASE_DIR};
use crate::security::identity::IdentityManager;
use crate::security::pqc::{MldsaVariant, PqcProvider};
use anyhow::{Result, anyhow};
use base64::{Engine as _, engine::general_purpose};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use pkcs8::{DecodePrivateKey, DecodePublicKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Debug)]
pub struct IntegrityManifest {
    pub binary_hash: String,
    pub source_hash: String,
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
    /// Recursively calculate the hash of the source code and metadata files.
    fn calculate_source_hash(&self) -> Result<String> {
        let mut files = Vec::new();

        // Check common locations
        let paths_to_check = vec![PathBuf::from("src"), PathBuf::from(".")];
        for path in paths_to_check {
            if !path.exists() {
                continue;
            }
            self.collect_files(&path, &mut files)?;
        }

        files.sort();
        files.dedup();

        let mut overall_hasher = Sha256::new();
        for path in files {
            let mut file = fs::File::open(&path)?;
            let mut file_hasher = Sha256::new();
            let mut buffer = [0u8; 8192];
            use std::io::Read;
            loop {
                let n = file.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                file_hasher.update(&buffer[..n]);
            }
            // Include path in the hash to detect moved files
            overall_hasher.update(path.to_string_lossy().as_bytes());
            overall_hasher.update(file_hasher.finalize());
        }

        Ok(crate::utils::hex_encode(overall_hasher.finalize()))
    }

    fn collect_files(&self, dir: &std::path::Path, files: &mut Vec<PathBuf>) -> Result<()> {
        if dir.is_file() {
            if let Some(ext) = dir.extension()
                && (ext == "rs" || ext == "toml")
            {
                files.push(dir.to_path_buf());
            }
            return Ok(());
        }

        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    // Skip hidden directories and target
                    if let Some(name) = path.file_name().and_then(|n| n.to_str())
                        && (name.starts_with('.') || name == "target")
                    {
                        continue;
                    }
                    self.collect_files(&path, files)?;
                } else if let Some(ext) = path.extension()
                    && (ext == "rs" || ext == "toml")
                {
                    files.push(path);
                }
            }
        }
        Ok(())
    }

    /// Calculates a combined integrity hash of all tool binaries.
    /// This uses SHA-256 as the underlying primitive, which is then signed by ML-DSA.
    /// In a post-quantum context, this hash acts as the message digest for the ML-DSA signature.
    fn calculate_binary_hash(&self) -> Result<String> {
        let exe_path = std::env::current_exe()?;
        let bin_dir = exe_path
            .parent()
            .ok_or_else(|| anyhow!("Could not get binary directory"))?;

        #[cfg(windows)]
        let tool_binaries = ["llsc.exe"];
        #[cfg(not(windows))]
        let tool_binaries = ["llsc"];

        let mut found_any = false;
        let mut overall_hasher = Sha256::new();

        let mut sorted_tools = tool_binaries.to_vec();
        sorted_tools.sort();

        for bin_name in sorted_tools {
            let bin_path = bin_dir.join(bin_name);
            if bin_path.exists() && bin_path.is_file() {
                let mut file = fs::File::open(&bin_path)?;
                let mut file_hasher = Sha256::new();

                let mut buffer = [0u8; 8192];
                use std::io::Read;
                loop {
                    let n = file.read(&mut buffer)?;
                    if n == 0 {
                        break;
                    }
                    file_hasher.update(&buffer[..n]);
                }

                // Combine file hashes into the overall digest
                overall_hasher.update(file_hasher.finalize());
                found_any = true;
            }
        }

        if !found_any {
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
            return Ok(crate::utils::hex_encode(hasher.finalize()));
        }

        Ok(crate::utils::hex_encode(overall_hasher.finalize()))
    }

    /// Calculates the SHA-256 hash of the configuration file.
    fn calculate_config_hash(&self) -> Result<String> {
        if !CONFIG_FILE_PATH.exists() {
            return Ok("MISSING".to_string());
        }
        let mut file = fs::File::open(&*CONFIG_FILE_PATH)?;
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
        let source_hash = self.calculate_source_hash()?;
        let config_hash = self.calculate_config_hash()?;

        let mut data_to_sign = BTreeMap::new();
        data_to_sign.insert("binary_hash", &binary_hash);
        data_to_sign.insert("source_hash", &source_hash);
        data_to_sign.insert("config_hash", &config_hash);
        let json_data = serde_json::to_string(&data_to_sign)?;

        // 1. Sign with PQC (ML-DSA)
        let sk_pqc = IdentityManager::get_pqc_private_key(MldsaVariant::Mldsa65)
            .map_err(|_| anyhow!("Identity keys not found. Run 'keygen' first."))?;
        let pqc_sig =
            PqcProvider::sign_mldsa(json_data.as_bytes(), &sk_pqc, MldsaVariant::Mldsa65)?;

        // 2. Sign with Ed25519 (Classical)
        let classical_priv_pem = IdentityManager::get_classical_private_key_pem()?;
        let signing_key = SigningKey::from_pkcs8_pem(&classical_priv_pem)
            .map_err(|e| anyhow!("Failed to load Ed25519 private key: {}", e))?;
        let classical_sig = signing_key.sign(json_data.as_bytes());

        let manifest = IntegrityManifest {
            binary_hash,
            source_hash,
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
        let mut data_to_verify = BTreeMap::new();
        data_to_verify.insert("binary_hash", &manifest.binary_hash);
        data_to_verify.insert("source_hash", &manifest.source_hash);
        data_to_verify.insert("config_hash", &manifest.config_hash);
        let json_data = serde_json::to_string(&data_to_verify)?;

        let pk_pqc = IdentityManager::get_pqc_public_key(MldsaVariant::Mldsa65)?;
        let signature_pqc = general_purpose::STANDARD.decode(&manifest.pqc_signature)?;

        if !PqcProvider::verify_mldsa(
            json_data.as_bytes(),
            &signature_pqc,
            &pk_pqc,
            MldsaVariant::Mldsa65,
        ) {
            return Ok(false); // PQC signature mismatch
        }

        // 1b. Verify Classical Signature (Ed25519)
        if let Some(classical_sig_b64) = &manifest.classical_signature {
            let classical_pub_path = crate::consts::KEY_DIR.join("id_ed25519.pub");
            let classical_pub_pem = fs::read_to_string(classical_pub_path)?;
            let verifying_key = VerifyingKey::from_public_key_pem(&classical_pub_pem)
                .map_err(|e| anyhow!("Failed to load Ed25519 public key: {}", e))?;
            let classical_sig_bytes = general_purpose::STANDARD.decode(classical_sig_b64)?;
            let classical_sig = Signature::from_slice(&classical_sig_bytes)
                .map_err(|e| anyhow!("Invalid Ed25519 signature format: {}", e))?;

            if verifying_key
                .verify(json_data.as_bytes(), &classical_sig)
                .is_err()
            {
                return Ok(false); // Ed25519 signature mismatch
            }
        }

        // 2. Compare current file hashes with the manifest
        let current_binary = self.calculate_binary_hash()?;
        if current_binary != manifest.binary_hash {
            return Ok(false); // Binary has been modified
        }

        let current_source = self.calculate_source_hash()?;
        if current_source != manifest.source_hash {
            return Ok(false); // Source files have been modified or new files added
        }

        let current_config = self.calculate_config_hash()?;
        if current_config != manifest.config_hash {
            return Ok(false); // Configuration has been modified
        }

        Ok(true)
    }
}
