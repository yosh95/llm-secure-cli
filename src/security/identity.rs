use crate::security::pqc::{DEFAULT_KEM_VARIANT, DEFAULT_PQC_VARIANT, PQCVariant, PqcProvider};
use anyhow::{Result, anyhow};
use std::fs;
use std::path::Path;
use std::path::PathBuf;

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
/// Key files are stored under `~/.llsc/keys/{entity_type}/{name}/`.
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
        super::key_storage::purge_passphrase_cache();
    }
}

// ─────────────────────────────────────────────
// IdentityManager
// ─────────────────────────────────────────────

pub struct IdentityManager;

impl IdentityManager {
    fn get_base_dir() -> PathBuf {
        crate::consts::key_dir()
    }

    fn get_key_dir(entity_type: &str, name: &str) -> PathBuf {
        Self::get_base_dir().join(entity_type).join(name)
    }

    // ── Key existence check ──

    /// Check whether both the ML-DSA and ML-KEM identity keys exist.
    #[must_use]
    pub fn has_keys() -> bool {
        let dir = Self::get_key_dir("self", "me");
        dir.join(DEFAULT_PQC_VARIANT.key_filename()).exists()
            && dir.join(DEFAULT_KEM_VARIANT.key_filename()).exists()
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
        if dir.join(DEFAULT_PQC_VARIANT.key_filename()).exists()
            && dir.join(DEFAULT_KEM_VARIANT.key_filename()).exists()
        {
            return Ok(());
        }

        // Acquire passphrase (may be None for no encryption)
        let passphrase = super::key_storage::read_optional_passphrase()?;

        // ML-DSA (FIPS 204) — post-quantum signing key
        let pqc_path = dir.join(DEFAULT_PQC_VARIANT.key_filename());
        if !pqc_path.exists() {
            let (pk, sk) =
                PqcProvider::generate_keypair(crate::security::pqc::DEFAULT_PQC_VARIANT)?;
            store.save_private_key(&pqc_path, &sk, passphrase.as_deref())?;
            fs::write(dir.join(DEFAULT_PQC_VARIANT.pub_key_filename()), pk)?;
        }

        // ML-KEM (FIPS 203) — post-quantum key encapsulation
        let kem_path = dir.join(DEFAULT_KEM_VARIANT.key_filename());
        if !kem_path.exists() {
            let (pk, sk) =
                PqcProvider::generate_kem_keypair(crate::security::pqc::DEFAULT_KEM_VARIANT)?;
            store.save_private_key(&kem_path, &sk, passphrase.as_deref())?;
            fs::write(dir.join(DEFAULT_KEM_VARIANT.pub_key_filename()), &pk)?;
        }

        Ok(())
    }

    /// Generate all identity keys using a custom [`KeyStore`] and an explicit passphrase.
    ///
    /// This method is primarily intended for testing, where you want to control
    /// the passphrase without interactive prompts or environment variables.
    /// Pass `None` for unencrypted (raw) keys.
    pub fn ensure_keys_with_passphrase(passphrase: Option<&str>) -> Result<()> {
        let store = FileSystemKeyStore;
        Self::ensure_keys_with_store_and_passphrase(&store, passphrase)
    }

    /// Generate all identity keys using a custom [`KeyStore`] and an explicit passphrase.
    pub fn ensure_keys_with_store_and_passphrase(
        store: &dyn KeyStore,
        passphrase: Option<&str>,
    ) -> Result<()> {
        let dir = store.base_dir().join("self").join("me");
        if !dir.exists() {
            fs::create_dir_all(&dir)?;
        }

        // If keys already exist, do nothing.
        if dir.join(DEFAULT_PQC_VARIANT.key_filename()).exists()
            && dir.join(DEFAULT_KEM_VARIANT.key_filename()).exists()
        {
            return Ok(());
        }

        // ML-DSA (FIPS 204) — post-quantum signing key
        let pqc_path = dir.join(DEFAULT_PQC_VARIANT.key_filename());
        if !pqc_path.exists() {
            let (pk, sk) =
                PqcProvider::generate_keypair(crate::security::pqc::DEFAULT_PQC_VARIANT)?;
            store.save_private_key(&pqc_path, &sk, passphrase)?;
            fs::write(dir.join(DEFAULT_PQC_VARIANT.pub_key_filename()), pk)?;
        }

        // ML-KEM (FIPS 203) — post-quantum key encapsulation
        let kem_path = dir.join(DEFAULT_KEM_VARIANT.key_filename());
        if !kem_path.exists() {
            let (pk, sk) =
                PqcProvider::generate_kem_keypair(crate::security::pqc::DEFAULT_KEM_VARIANT)?;
            store.save_private_key(&kem_path, &sk, passphrase)?;
            fs::write(dir.join(DEFAULT_KEM_VARIANT.pub_key_filename()), &pk)?;
        }

        Ok(())
    }

    // ── Public key reads ──

    /// Read any public key file from the identity key directory.
    pub fn get_public_key_for(entity_type: &str, name: &str, filename: &str) -> Result<Vec<u8>> {
        let path = Self::get_key_dir(entity_type, name).join(filename);
        if !path.exists() {
            return Err(anyhow!("Public key not found: {path:?}"));
        }
        Ok(fs::read(path)?)
    }

    /// Read the ML-DSA public key for the default identity.
    pub fn get_pqc_public_key(_variant: PQCVariant) -> Result<Vec<u8>> {
        Self::get_public_key_for("self", "me", &_variant.pub_key_filename())
    }

    /// Read the ML-KEM public key for the default identity.
    pub fn get_kem_public_key() -> Result<Vec<u8>> {
        Ok(fs::read(
            Self::get_key_dir("self", "me").join(DEFAULT_KEM_VARIANT.pub_key_filename()),
        )?)
    }

    // ── Private key reads ──

    /// Load an ML-DSA private key (raw or encrypted).
    ///
    /// Uses the default [`FileSystemKeyStore`].
    pub fn get_pqc_private_key(_variant: PQCVariant) -> Result<Vec<u8>> {
        let store = FileSystemKeyStore;
        Self::get_pqc_private_key_with_store(&store, crate::security::pqc::DEFAULT_PQC_VARIANT)
    }

    /// Load an ML-DSA private key using a custom [`KeyStore`].
    pub fn get_pqc_private_key_with_store(
        store: &dyn KeyStore,
        _variant: PQCVariant,
    ) -> Result<Vec<u8>> {
        let path = store
            .base_dir()
            .join("self")
            .join("me")
            .join(_variant.key_filename());
        store.load_private_key(&path)
    }

    /// Load the ML-KEM private key (raw or encrypted).
    ///
    /// Uses the default [`FileSystemKeyStore`].
    pub fn get_kem_private_key() -> Result<Vec<u8>> {
        let store = FileSystemKeyStore;
        Self::get_kem_private_key_with_store(&store)
    }

    /// Load the ML-KEM private key using a custom [`KeyStore`].
    pub fn get_kem_private_key_with_store(store: &dyn KeyStore) -> Result<Vec<u8>> {
        let path = store
            .base_dir()
            .join("self")
            .join("me")
            .join(DEFAULT_KEM_VARIANT.key_filename());
        store.load_private_key(&path)
    }
}
