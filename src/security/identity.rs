use crate::consts::KEY_DIR;
use crate::security::pqc::{MldsaVariant, MlkemVariant, PQCAgilityManager, PqcProvider};
use crate::security::pqc_cose::HybridSigner;
use anyhow::Result;
use base64::{Engine as _, engine::general_purpose};
use chrono::Utc;
use ed25519_dalek::SigningKey;
use once_cell::sync::Lazy;
use pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use rand::rngs::OsRng;
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use uuid::Uuid;

pub struct IdentityManager;

static KEY_CACHE: Lazy<Mutex<HashMap<String, Vec<u8>>>> = Lazy::new(|| Mutex::new(HashMap::new()));
static KEYS_ENSURED: Lazy<Mutex<bool>> = Lazy::new(|| Mutex::new(false));

impl IdentityManager {
    const PRIVATE_KEY_PATH: &str = "id_ed25519";
    const PUBLIC_KEY_PATH: &str = "id_ed25519.pub";
    const PQC_KEM_PRIVATE_KEY_PATH: &str = "id_kem.key";
    const PQC_KEM_PUBLIC_KEY_PATH: &str = "id_kem.pub";

    fn get_pqc_paths(variant: MldsaVariant) -> (PathBuf, PathBuf) {
        let (priv_name, pub_name) = match variant {
            MldsaVariant::Mldsa44 => ("id_pqc_l2.key", "id_pqc_l2.pub"),
            MldsaVariant::Mldsa65 => ("id_pqc_l3.key", "id_pqc_l3.pub"),
            MldsaVariant::Mldsa87 => ("id_pqc_l5.key", "id_pqc_l5.pub"),
        };
        (KEY_DIR.join(priv_name), KEY_DIR.join(pub_name))
    }

    pub fn ensure_keys(force: bool) -> Result<()> {
        let mut ensured = KEYS_ENSURED.lock().unwrap();
        if *ensured && !force {
            return Ok(());
        }

        if !KEY_DIR.exists() {
            fs::create_dir_all(&*KEY_DIR)?;
        }

        // Classical Ed25519 Keys
        let ed_priv_path = KEY_DIR.join(Self::PRIVATE_KEY_PATH);
        let ed_pub_path = KEY_DIR.join(Self::PUBLIC_KEY_PATH);

        if force || !ed_priv_path.exists() || !ed_pub_path.exists() {
            let mut rng = OsRng;
            let signing_key = SigningKey::generate(&mut rng);
            let verifying_key = signing_key.verifying_key();

            let priv_pem = signing_key.to_pkcs8_pem(LineEnding::LF)?;
            let pub_pem = verifying_key.to_public_key_pem(LineEnding::LF)?;

            Self::write_private_file(&ed_priv_path, priv_pem.as_bytes())?;
            Self::write_public_file(&ed_pub_path, pub_pem.as_bytes())?;
        }

        // ML-DSA Keys
        for variant in [
            MldsaVariant::Mldsa44,
            MldsaVariant::Mldsa65,
            MldsaVariant::Mldsa87,
        ] {
            let (priv_p, pub_p) = Self::get_pqc_paths(variant);
            if force || !priv_p.exists() || !pub_p.exists() {
                let (pk, sk) = PqcProvider::generate_mldsa_keypair(variant);
                Self::write_private_file(&priv_p, &sk)?;
                Self::write_public_file(&pub_p, &pk)?;
            }
        }

        // ML-KEM Keys
        let kem_priv_path = KEY_DIR.join(Self::PQC_KEM_PRIVATE_KEY_PATH);
        let kem_pub_path = KEY_DIR.join(Self::PQC_KEM_PUBLIC_KEY_PATH);
        if force || !kem_priv_path.exists() || !kem_pub_path.exists() {
            let (pk, sk) = PqcProvider::generate_mlkem_keypair(MlkemVariant::Mlkem768);
            Self::write_private_file(&kem_priv_path, &sk)?;
            Self::write_public_file(&kem_pub_path, &pk)?;
        }

        *ensured = true;
        if force {
            KEY_CACHE.lock().unwrap().clear();
        }
        Ok(())
    }

    pub fn has_keys() -> bool {
        let ed_priv_path = KEY_DIR.join(Self::PRIVATE_KEY_PATH);
        let (pqc_priv_path, _) = Self::get_pqc_paths(MldsaVariant::Mldsa65);
        ed_priv_path.exists() && pqc_priv_path.exists()
    }

    fn write_private_file(path: &Path, content: &[u8]) -> Result<()> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);

        let mut file = options.open(path)?;
        file.write_all(content)?;
        Ok(())
    }

    fn write_public_file(path: &Path, content: &[u8]) -> Result<()> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);

        let mut file = options.open(path)?;
        file.write_all(content)?;
        Ok(())
    }

    pub fn get_classical_private_key_pem() -> Result<String> {
        Self::ensure_keys(false)?;
        let path = KEY_DIR.join(Self::PRIVATE_KEY_PATH);
        Ok(fs::read_to_string(path)?)
    }

    pub fn get_pqc_private_key(variant: MldsaVariant) -> Result<Vec<u8>> {
        Self::ensure_keys(false)?;
        let (path, _) = Self::get_pqc_paths(variant);
        Ok(fs::read(path)?)
    }

    pub fn get_pqc_public_key(variant: MldsaVariant) -> Result<Vec<u8>> {
        Self::ensure_keys(false)?;
        let (_, path) = Self::get_pqc_paths(variant);
        Ok(fs::read(path)?)
    }

    pub fn get_kem_public_key() -> Result<Vec<u8>> {
        Self::ensure_keys(false)?;
        let path = KEY_DIR.join(Self::PQC_KEM_PUBLIC_KEY_PATH);
        Ok(fs::read(path)?)
    }

    pub fn get_kem_private_key() -> Result<Vec<u8>> {
        Self::ensure_keys(false)?;
        let path = KEY_DIR.join(Self::PQC_KEM_PRIVATE_KEY_PATH);
        Ok(fs::read(path)?)
    }

    pub fn get_local_identity() -> String {
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_else(|_| "unknown_user".to_string());
        let hostname = hostname::get()
            .map(|h| h.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "unknown_host".to_string());
        format!("{}@{}", user, hostname)
    }

    pub fn generate_token(
        config: &crate::config::models::AppConfig,
        user_id: Option<&str>,
        audience: Option<&str>,
        tool_name: Option<&str>,
        args: Option<&serde_json::Value>,
    ) -> Result<String> {
        let uid = user_id
            .map(|s| s.to_string())
            .unwrap_or_else(Self::get_local_identity);
        let now = Utc::now().timestamp();

        let mut payload = json!({
            "iss": "llm-cli-client",
            "sub": uid,
            "iat": now,
            "exp": now + 600,
            "jti": Uuid::new_v4().to_string(),
            "pqc": true,
            "pqc_kem_pub": general_purpose::STANDARD.encode(Self::get_kem_public_key()?),
        });

        if let Some(tool) = tool_name {
            payload["tool"] = json!(tool);
        }
        if let Some(aud) = audience {
            payload["aud"] = json!(aud);
        }

        let variant = if let Some(tool) = tool_name {
            let v = PQCAgilityManager::get_required_level(config, tool, args, "standard");
            log::debug!(
                "IdentityManager: Agility scaling for tool '{}': {:?}",
                tool,
                v
            );
            v
        } else {
            MldsaVariant::Mldsa65
        };

        log::debug!(
            "IdentityManager: Generating identity token for subject '{}' (PQC: {:?})",
            uid,
            variant
        );
        let classical_priv = Self::get_classical_private_key_pem()?;
        let pqc_priv = Self::get_pqc_private_key(variant)?;

        let cose_token_bytes =
            HybridSigner::create_hybrid_token(&payload, &classical_priv, &pqc_priv, variant);

        Ok(general_purpose::URL_SAFE_NO_PAD.encode(cose_token_bytes))
    }
}
