use crate::security::pqc::{PQCVariant, PqcProvider};
use crate::security::pqc_cose::HybridSigner;
use anyhow::{Result, anyhow};
use chrono::Utc;
use ed25519_dalek::SigningKey;
use fips203::traits::KeyGen;
use fips203::traits::SerDes;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use uuid::Uuid;

// ─────────────────────────────────────────────
// KeyStore trait: abstraction for key management
// ─────────────────────────────────────────────

/// Trait abstracting key storage operations.
///
/// The default implementation (`FileSystemKeyStore`) stores keys on the local
/// filesystem with optional AES-256-GCM + Argon2id encryption.  Alternative
/// implementations can backed by HSMs, cloud KMS, or secrets managers.
///
/// # Security Considerations
///
/// - Private keys **must** never leave the store unencrypted (except during
///   signing/decryption operations that happen in-process).
/// - Implementations should be `Send + Sync` so they can be shared across
///   async tasks.
/// - The `drop_keys` method provides an explicit in-memory zeroisation hook
///   for stores that cache decrypted key material.
pub trait KeyStore: Send + Sync {
    /// Save a private key, optionally encrypting it with `passphrase`.
    fn save_private_key(
        &self,
        path: &Path,
        key_bytes: &[u8],
        passphrase: Option<&str>,
    ) -> Result<()>;

    /// Load a private key. Transparently handles encrypted (LKEF) keys
    /// by prompting or reading the passphrase.
    fn load_private_key(&self, path: &Path) -> Result<Vec<u8>>;

    /// Check whether a key file is encrypted (starts with LKEF magic).
    fn is_encrypted(&self, path: &Path) -> bool;

    /// Return the base directory for key storage.
    fn base_dir(&self) -> PathBuf;

    /// Purge any in-memory cached secrets (passphrase cache, decrypted keys).
    /// Called on session end to minimise the window of exposure.
    fn drop_cache(&self);
}

// ─────────────────────────────────────────────
// FileSystemKeyStore: production implementation
// ─────────────────────────────────────────────

/// Default key store backed by the local filesystem.
///
/// Key files are stored under `~/.llm_secure_cli/keys/{entity_type}/{name}/`.
/// Optional passphrase-based encryption uses AES-256-GCM with an Argon2id
/// key derivation function.
pub struct FileSystemKeyStore;

impl KeyStore for FileSystemKeyStore {
    fn save_private_key(
        &self,
        path: &Path,
        key_bytes: &[u8],
        passphrase: Option<&str>,
    ) -> Result<()> {
        super::key_storage::save_key(path, key_bytes, passphrase)
    }

    fn load_private_key(&self, path: &Path) -> Result<Vec<u8>> {
        super::key_storage::load_key(path)
    }

    fn is_encrypted(&self, path: &Path) -> bool {
        super::key_storage::is_encrypted(path)
    }

    fn base_dir(&self) -> PathBuf {
        crate::consts::key_dir()
    }

    fn drop_cache(&self) {
        super::key_storage::purge_passphrase_cache()
    }
}

// ─────────────────────────────────────────────
// IdentityClaims & IdentityManager
// ─────────────────────────────────────────────

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

    // ── Key existence check ──

    pub fn has_keys() -> bool {
        // Accept both raw and LKEF-magic encrypted key files.
        let dir = Self::get_key_dir("self", "me");
        (dir.join("id_ed25519").exists()) && (dir.join("id_mldsa87").exists())
    }

    // ── Key generation ──

    /// Generate all identity keys. Prompts for an optional passphrase
    /// (interactive only) which, if provided, encrypts all private keys
    /// with AES-256-GCM (Argon2id KDF).
    ///
    /// Uses the default [`FileSystemKeyStore`] for persistence.
    pub fn ensure_keys() -> Result<()> {
        let store = FileSystemKeyStore;
        Self::ensure_keys_with_store(&store)
    }

    /// Generate all identity keys using a custom [`KeyStore`] implementation.
    ///
    /// This method enables plugging in alternative key storage backends
    /// (HSM, cloud KMS, secrets manager) without modifying `IdentityManager`.
    pub fn ensure_keys_with_store(store: &dyn KeyStore) -> Result<()> {
        let dir = store.base_dir().join("self").join("me");
        if !dir.exists() {
            fs::create_dir_all(&dir)?;
        }

        // If keys already exist, do nothing.
        if dir.join("id_ed25519").exists() && dir.join("id_mldsa87").exists() {
            return Ok(());
        }

        // Acquire passphrase (may be None for no encryption)
        let passphrase = super::key_storage::read_optional_passphrase()?;

        // Ed25519
        let ed_path = dir.join("id_ed25519");
        if !ed_path.exists() {
            let mut rng = OsRng;
            let signing_key = SigningKey::generate(&mut rng);
            let priv_bytes = signing_key.to_bytes();
            let pub_bytes = signing_key.verifying_key().to_bytes();

            store.save_private_key(&ed_path, &priv_bytes, passphrase.as_deref())?;
            fs::write(dir.join("id_ed25519.pub"), pub_bytes)?;
        }

        // ML-DSA variants
        let pqc_variants = [
            PQCVariant::MLDSA44,
            PQCVariant::MLDSA65,
            PQCVariant::MLDSA87,
        ];
        for variant in pqc_variants {
            let pqc_path = dir.join(match variant {
                PQCVariant::MLDSA44 => "id_mldsa44",
                PQCVariant::MLDSA65 => "id_mldsa65",
                PQCVariant::MLDSA87 => "id_mldsa87",
            });
            if !pqc_path.exists() {
                let (pk, sk) = PqcProvider::generate_keypair(variant)?;
                store.save_private_key(&pqc_path, &sk, passphrase.as_deref())?;

                let pub_filename = format!(
                    "id_mldsa{}.pub",
                    match variant {
                        PQCVariant::MLDSA44 => "44",
                        PQCVariant::MLDSA65 => "65",
                        PQCVariant::MLDSA87 => "87",
                    }
                );
                fs::write(dir.join(pub_filename), pk)?;
            }
        }

        // ML-KEM (FIPS 203)
        let kem_path = dir.join("id_kem768");
        if !kem_path.exists() {
            let (pk, sk) = fips203::ml_kem_768::KG::try_keygen()
                .map_err(|e| anyhow!("KEM keygen failed: {}", e))?;
            store.save_private_key(&kem_path, &sk.into_bytes(), passphrase.as_deref())?;
            fs::write(dir.join("id_kem768.pub"), pk.into_bytes())?;
        }

        Ok(())
    }

    // ── Public key reads (unchanged — public keys are never encrypted) ──

    pub fn get_public_key_for(entity_type: &str, name: &str, filename: &str) -> Result<Vec<u8>> {
        let path = Self::get_key_dir(entity_type, name).join(filename);
        if !path.exists() {
            return Err(anyhow!("Public key not found: {:?}", path));
        }
        Ok(fs::read(path)?)
    }

    pub fn get_classical_public_key() -> Result<Vec<u8>> {
        Self::get_public_key_for("self", "me", "id_ed25519.pub")
    }

    pub fn get_pqc_public_key(variant: PQCVariant) -> Result<Vec<u8>> {
        let filename = match variant {
            PQCVariant::MLDSA44 => "id_mldsa44.pub",
            PQCVariant::MLDSA65 => "id_mldsa65.pub",
            PQCVariant::MLDSA87 => "id_mldsa87.pub",
        };
        Self::get_public_key_for("self", "me", filename)
    }

    pub fn get_kem_public_key() -> Result<Vec<u8>> {
        Ok(fs::read(
            Self::get_key_dir("self", "me").join("id_kem768.pub"),
        )?)
    }

    // ── Private key reads — routed through key_storage::load_key ──

    /// Load the Ed25519 private key (raw or encrypted), returned as PKCS#8 PEM.
    ///
    /// Uses the default [`FileSystemKeyStore`].
    pub fn get_classical_private_key_pem() -> Result<String> {
        let store = FileSystemKeyStore;
        Self::get_classical_private_key_pem_with_store(&store)
    }

    /// Load the Ed25519 private key using a custom [`KeyStore`].
    pub fn get_classical_private_key_pem_with_store(store: &dyn KeyStore) -> Result<String> {
        let path = store.base_dir().join("self").join("me").join("id_ed25519");
        let bytes = store.load_private_key(&path)?;
        let key = SigningKey::from_bytes(bytes.as_slice().try_into()?);
        use pkcs8::EncodePrivateKey;
        Ok(key.to_pkcs8_pem(pkcs8::LineEnding::LF)?.to_string())
    }

    /// Load an ML-DSA private key (raw or encrypted).
    ///
    /// Uses the default [`FileSystemKeyStore`].
    pub fn get_pqc_private_key(variant: PQCVariant) -> Result<Vec<u8>> {
        let store = FileSystemKeyStore;
        Self::get_pqc_private_key_with_store(&store, variant)
    }

    /// Load an ML-DSA private key using a custom [`KeyStore`].
    pub fn get_pqc_private_key_with_store(
        store: &dyn KeyStore,
        variant: PQCVariant,
    ) -> Result<Vec<u8>> {
        let filename = match variant {
            PQCVariant::MLDSA44 => "id_mldsa44",
            PQCVariant::MLDSA65 => "id_mldsa65",
            PQCVariant::MLDSA87 => "id_mldsa87",
        };
        let path = store.base_dir().join("self").join("me").join(filename);
        store.load_private_key(&path)
    }

    /// Load the ML-KEM-768 private key (raw or encrypted).
    ///
    /// Uses the default [`FileSystemKeyStore`].
    pub fn get_kem_private_key() -> Result<Vec<u8>> {
        let store = FileSystemKeyStore;
        Self::get_kem_private_key_with_store(&store)
    }

    /// Load the ML-KEM-768 private key using a custom [`KeyStore`].
    pub fn get_kem_private_key_with_store(store: &dyn KeyStore) -> Result<Vec<u8>> {
        let path = store.base_dir().join("self").join("me").join("id_kem768");
        store.load_private_key(&path)
    }

    // ── Token generation ──

    pub fn generate_token(tool_name: Option<&str>) -> Result<String> {
        Self::ensure_keys()?;

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

        // Read private keys via KeyStore (handles encryption transparently)
        let store = FileSystemKeyStore;
        let ed_sk =
            store.load_private_key(&store.base_dir().join("self").join("me").join("id_ed25519"))?;
        let pqc_sk =
            store.load_private_key(&store.base_dir().join("self").join("me").join("id_mldsa87"))?;

        // Create Hybrid COSE Token
        let cose_token = HybridSigner::create_hybrid_token(
            &payload,
            &ed_sk,
            &pqc_sk,
            crate::security::pqc::DEFAULT_PQC_VARIANT,
        )?;

        // Base64url encode for transport
        Ok(base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            cose_token,
        ))
    }
}
