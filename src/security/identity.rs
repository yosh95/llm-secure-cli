use crate::security::pqc::{PQCVariant, PqcProvider};
use crate::security::pqc_cose::HybridSigner;
use anyhow::{Result, anyhow};
use chrono::Utc;
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Debug)]
pub struct IdentityClaims {
    pub iss: String,
    pub sub: String,
    pub iat: i64,
    pub exp: i64,
    pub jti: String,
    pub tool: Option<String>,
    pub workspace: String,
}

pub struct IdentityManager;

impl IdentityManager {
    fn get_base_dir() -> PathBuf {
        crate::consts::key_dir()
    }

    fn get_key_dir(entity_type: &str, name: &str) -> PathBuf {
        Self::get_base_dir().join(entity_type).join(name)
    }

    pub fn ensure_keys() -> Result<()> {
        let dir = Self::get_key_dir("self", "me");
        if !dir.exists() {
            fs::create_dir_all(&dir)?;
        }

        let ed_path = dir.join("id_ed25519");
        if !ed_path.exists() {
            let mut rng = OsRng;
            let signing_key = SigningKey::generate(&mut rng);
            let priv_bytes = signing_key.to_bytes();
            let pub_bytes = signing_key.verifying_key().to_bytes();
            fs::write(&ed_path, priv_bytes)?;
            fs::write(dir.join("id_ed25519.pub"), pub_bytes)?;
        }

        let pqc_variants = [
            PQCVariant::MLDSA44,
            PQCVariant::MLDSA65,
            PQCVariant::MLDSA87,
        ];
        for variant in pqc_variants {
            let filename = match variant {
                PQCVariant::MLDSA44 => "id_mldsa44",
                PQCVariant::MLDSA65 => "id_mldsa65",
                PQCVariant::MLDSA87 => "id_mldsa87",
            };
            let pqc_path = dir.join(filename);
            if !pqc_path.exists() {
                let (pk, sk) = PqcProvider::generate_keypair(variant)?;
                fs::write(&pqc_path, sk)?;
                fs::write(dir.join(format!("{}.pub", filename)), pk)?;
            }
        }

        let kem_path = dir.join("id_kem768");
        if !kem_path.exists() {
            let v = saorsa_pqc::api::MlKemVariant::MlKem768;
            let ops = saorsa_pqc::api::MlKem::new(v);
            let (pk, sk) = ops
                .generate_keypair()
                .map_err(|_| anyhow!("KEM keygen failed"))?;
            fs::write(&kem_path, sk.to_bytes())?;
            fs::write(dir.join("id_kem768.pub"), pk.to_bytes())?;
        }

        Ok(())
    }

    pub fn get_public_key_for(entity_type: &str, name: &str, filename: &str) -> Result<Vec<u8>> {
        let path = Self::get_key_dir(entity_type, name).join(filename);
        if !path.exists() {
            return Err(anyhow!("Public key not found: {:?}", path));
        }
        Ok(fs::read(path)?)
    }

    pub fn has_keys() -> bool {
        let dir = Self::get_key_dir("self", "me");
        dir.join("id_ed25519").exists() && dir.join("id_mldsa65").exists()
    }

    pub fn get_classical_private_key_pem() -> Result<String> {
        let path = Self::get_key_dir("self", "me").join("id_ed25519");
        let bytes = fs::read(path)?;
        let key = SigningKey::from_bytes(bytes.as_slice().try_into()?);
        use pkcs8::EncodePrivateKey;
        Ok(key.to_pkcs8_pem(pkcs8::LineEnding::LF)?.to_string())
    }

    pub fn get_classical_public_key() -> Result<Vec<u8>> {
        Self::get_public_key_for("self", "me", "id_ed25519.pub")
    }

    pub fn get_pqc_private_key(variant: PQCVariant) -> Result<Vec<u8>> {
        let filename = match variant {
            PQCVariant::MLDSA44 => "id_mldsa44",
            PQCVariant::MLDSA65 => "id_mldsa65",
            PQCVariant::MLDSA87 => "id_mldsa87",
        };
        Ok(fs::read(Self::get_key_dir("self", "me").join(filename))?)
    }

    pub fn get_pqc_public_key(variant: PQCVariant) -> Result<Vec<u8>> {
        let filename = match variant {
            PQCVariant::MLDSA44 => "id_mldsa44.pub",
            PQCVariant::MLDSA65 => "id_mldsa65.pub",
            PQCVariant::MLDSA87 => "id_mldsa87.pub",
        };
        Ok(fs::read(Self::get_key_dir("self", "me").join(filename))?)
    }

    pub fn get_kem_private_key() -> Result<Vec<u8>> {
        Ok(fs::read(Self::get_key_dir("self", "me").join("id_kem768"))?)
    }

    pub fn get_kem_public_key() -> Result<Vec<u8>> {
        Ok(fs::read(
            Self::get_key_dir("self", "me").join("id_kem768.pub"),
        )?)
    }

    pub fn generate_token(tool_name: Option<&str>) -> Result<String> {
        Self::ensure_keys()?;
        let dir = Self::get_key_dir("self", "me");

        let sub = format!(
            "{}@{}",
            std::env::var("USER").unwrap_or_else(|_| "unknown".into()),
            hostname::get()?.to_string_lossy()
        );

        let now = Utc::now().timestamp();
        let claims = IdentityClaims {
            iss: "llsc-client".to_string(),
            sub,
            iat: now,
            exp: now + 600,
            jti: Uuid::new_v4().to_string(),
            tool: tool_name.map(|s| s.to_string()),
            workspace: format!("{:?}", std::env::current_dir()?),
        };

        // Serialize claims to CBOR for the COSE payload
        let mut payload = Vec::new();
        ciborium::into_writer(&claims, &mut payload)?;

        // Read private keys
        let ed_sk = fs::read(dir.join("id_ed25519"))?;
        let pqc_sk = fs::read(dir.join("id_mldsa65"))?;

        // Create Hybrid COSE Token
        let cose_token =
            HybridSigner::create_hybrid_token(&payload, &ed_sk, &pqc_sk, PQCVariant::MLDSA65)?;

        // Base64url encode for transport
        Ok(base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            cose_token,
        ))
    }
}
