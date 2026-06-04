use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use anyhow::{Result, anyhow};
use argon2::Argon2;
use rand::RngCore;
use std::fs;
use std::io::{IsTerminal, stdin};
use std::path::Path;
use std::sync::Mutex;
use zeroize::{Zeroize, Zeroizing};

/// Header magic bytes to identify encrypted key files.
const ENCRYPTED_KEY_MAGIC: &[u8; 4] = b"LKEF";
const HEADER_SIZE: usize = 4 + 16 + 12; // magic(4) + salt(16) + nonce(12) = 32

/// Per-key-path passphrase cache for the current session.
///
/// Maps key file paths to their passphrases so that multiple keys with
/// different passphrases can be used in the same session without repeated
/// prompts.  Uses `Zeroizing<String>` to ensure passphrases are zeroed
/// in memory on drop.
///
/// The cache is cleared by [`purge_passphrase_cache`] when the session ends
/// (via [`crate::core::session::ActiveSession`]'s `Drop` or `close()`).
/// Per-key-path passphrase cache for the current session.
///
/// Uses `std::sync::LazyLock` for lazy initialization since
/// `HashMap::new()` is not `const`.
static PASSPHRASE_CACHE: std::sync::LazyLock<
    Mutex<std::collections::HashMap<std::path::PathBuf, Zeroizing<String>>>,
> = std::sync::LazyLock::new(|| Mutex::new(std::collections::HashMap::new()));

// ─────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────

/// Persist private key material to disk, optionally encrypted with a passphrase.
///
/// If `passphrase` is `Some` and non-empty, the key is encrypted with AES-256-GCM
/// using an Argon2id-derived key. If `passphrase` is `None` or empty, the key is
/// stored as raw bytes (backwards compatible).
pub fn save_key(path: &Path, key_bytes: &[u8], passphrase: Option<&str>) -> Result<()> {
    match passphrase {
        Some(pw) if !pw.is_empty() => save_encrypted(path, key_bytes, pw),
        _ => {
            fs::write(path, key_bytes)?;
            set_permissions(path)?;
            Ok(())
        }
    }
}

/// Load a private key from disk.
///
/// If the file starts with the `LKEF` magic header it is treated as encrypted
/// and the passphrase is requested (interactive or env var). Otherwise the raw
/// bytes are returned directly.
pub fn load_key(path: &Path) -> Result<Vec<u8>> {
    if !path.exists() {
        return Err(anyhow!("Key file not found: {path:?}"));
    }

    let mut file = fs::File::open(path)?;
    let mut magic = [0u8; 4];
    use std::io::Read;
    // If the file is too short or read fails, treat as raw (backward compat).
    if file.read_exact(&mut magic).is_err() || &magic != ENCRYPTED_KEY_MAGIC {
        // Fall back to reading the whole file as raw key bytes.
        return Ok(fs::read(path)?);
    }

    // Encrypted path — resolve passphrase from cache, env, or interactive prompt
    let mut data = Vec::new();
    data.extend_from_slice(&magic);
    file.read_to_end(&mut data)?;
    let pw = resolve_passphrase(path)?;
    load_encrypted_key_data(&data, &pw)
}

/// Resolve the passphrase for a given key file path.
///
/// Resolution order:
/// 1. Check the per-path cache (returns cached passphrase if previously entered).
/// 2. Prompt the user interactively (if stdin is a terminal).
/// 3. Fall back to the `LLM_CLI_KEY_PASSPHRASE` env var or `*_FILE` Docker secret.
///
/// On success, the passphrase is cached for the remainder of the session
/// (keyed by `path`) and also stored as the "default" for any subsequent key
/// that lacks its own cache entry.
fn resolve_passphrase(path: &Path) -> Result<String> {
    // 1. Check per-path cache
    {
        let cache = PASSPHRASE_CACHE
            .lock()
            .map_err(|_| anyhow!("Cache lock poisoned"))?;
        if let Some(pw) = cache.get(path) {
            return Ok(pw.to_string());
        }
        // Fallback to the first cached passphrase (for workflows that use
        // a single passphrase for all keys).
        if let Some(pw) = cache.values().next() {
            return Ok(pw.to_string());
        }
    }

    // 2. Check environment variable (overrides interactive prompt).
    //    This works whether or not stdin is a terminal.
    if let Ok(pw) = std::env::var("LLM_CLI_KEY_PASSPHRASE") {
        let pw = pw.trim().to_string();
        if pw.is_empty() {
            return Err(anyhow!(
                "LLM_CLI_KEY_PASSPHRASE is empty \u{2014} passphrase required"
            ));
        }
        // Cache for session
        {
            let mut cache = PASSPHRASE_CACHE
                .lock()
                .map_err(|_| anyhow!("Cache lock poisoned"))?;
            cache.insert(path.to_path_buf(), Zeroizing::new(pw.clone()));
        }
        return Ok(pw);
    }

    // 3. Check *_FILE variant (Docker secrets pattern)
    if let Ok(path_str) = std::env::var("LLM_CLI_KEY_PASSPHRASE_FILE")
        && let Ok(content) = fs::read_to_string(&path_str)
    {
        let pw = content.trim().to_string();
        if !pw.is_empty() {
            // Cache for session
            {
                let mut cache = PASSPHRASE_CACHE
                    .lock()
                    .map_err(|_| anyhow!("Cache lock poisoned"))?;
                cache.insert(path.to_path_buf(), Zeroizing::new(pw.clone()));
            }
            return Ok(pw);
        }
    }

    // 4. Not cached, no env var \u{2014} prompt interactively if on a terminal
    if !stdin().is_terminal() {
        return Err(anyhow!(
            "Encrypted PQC keys require a passphrase.\n                 For interactive use, run from a terminal.\n                 For non-interactive use, set LLM_CLI_KEY_PASSPHRASE environment variable."
        ));
    }
    let pw = read_passphrase_interactive()?;

    // 5. Cache for session (keyed by path)
    {
        let mut cache = PASSPHRASE_CACHE
            .lock()
            .map_err(|_| anyhow!("Cache lock poisoned"))?;
        cache.insert(path.to_path_buf(), Zeroizing::new(pw.clone()));
    }

    Ok(pw)
}
/// Purge the passphrase cache (called on session end).
///
/// This clears all cached passphrases from memory.  Called by
/// [`KeyStore::drop_cache`](crate::security::identity::KeyStore::drop_cache)
/// which is invoked during [`crate::core::session::ActiveSession`]
/// teardown (`close()` or `Drop`).
pub fn purge_passphrase_cache() {
    if let Ok(mut cache) = PASSPHRASE_CACHE.lock() {
        // Clear all entries; Zeroizing ensures automatic zeroization
        // of each passphrase as it is dropped.
        cache.clear();
    }
}

/// Returns true if the key file is encrypted (starts with LKEF magic).
#[must_use]
pub fn is_encrypted(path: &Path) -> bool {
    path.exists() && fs::read(path).is_ok_and(|data| data.starts_with(ENCRYPTED_KEY_MAGIC))
}

/// Prompts the user for an *optional* passphrase (empty = no encryption).
/// Returns `Ok(Some(pw))` for a non-empty passphrase, `Ok(None)` if empty.
/// In non-interactive mode, checks `LLM_CLI_KEY_PASSPHRASE` env var.
pub fn read_optional_passphrase() -> Result<Option<String>> {
    // Check environment variable first (overrides interactive prompt).
    // Empty string \u{2192} no encryption (raw keys); non-empty \u{2192} use as passphrase.
    if let Ok(pw) = std::env::var("LLM_CLI_KEY_PASSPHRASE") {
        return if pw.trim().is_empty() {
            Ok(None)
        } else {
            Ok(Some(pw.trim().to_string()))
        };
    }
    // Also check *_FILE variant (Docker secrets pattern)
    if let Ok(path) = std::env::var("LLM_CLI_KEY_PASSPHRASE_FILE")
        && let Ok(content) = fs::read_to_string(&path)
    {
        let pw = content.trim().to_string();
        if !pw.is_empty() {
            return Ok(Some(pw));
        }
    }

    // Fall back to interactive prompt if on a terminal
    if stdin().is_terminal() {
        read_optional_passphrase_interactive()
    } else {
        Ok(None) // Default: no passphrase in non-interactive mode
    }
} // Internal: interactive passphrase prompts
// ─────────────────────────────────────────────

fn read_optional_passphrase_interactive() -> Result<Option<String>> {
    // rpassword prints prompt to stderr and reads with echo disabled.
    // For an optional passphrase, empty input means "no passphrase".
    // Loop until passphrases match, or user provides an empty passphrase.
    loop {
        let pw = rpassword::prompt_password(
            "Enter passphrase for PQC keys (empty for no passphrase): ",
        )?;

        if pw.is_empty() {
            return Ok(None);
        }

        let confirm = rpassword::prompt_password("Confirm passphrase: ")?;

        if pw == confirm {
            return Ok(Some(pw));
        }

        eprintln!("Passphrases do not match. Please try again.");
    }
}

fn read_passphrase_interactive() -> Result<String> {
    let pw = rpassword::prompt_password("PQC key passphrase: ")?;

    if pw.is_empty() {
        return Err(anyhow!("Passphrase cannot be empty for encrypted keys"));
    }
    Ok(pw)
}

// ─────────────────────────────────────────────
// Internal: passphrase resolution with cache
// ─────────────────────────────────────────────

// ─────────────────────────────────────────────
// Internal: encryption / decryption
// ─────────────────────────────────────────────

fn save_encrypted(path: &Path, key_bytes: &[u8], passphrase: &str) -> Result<()> {
    // 1. Derive AES key via Argon2id
    let mut aes_key = [0u8; 32];
    let mut salt = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut salt);

    Argon2::default()
        .hash_password_into(passphrase.as_bytes(), &salt, &mut aes_key)
        .map_err(|e| anyhow!("Argon2id KDF failed: {e}"))?;

    // 2. AES-256-GCM encrypt
    let cipher = Aes256Gcm::new_from_slice(&aes_key).map_err(|_| anyhow!("AES init failed"))?;

    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from(nonce_bytes);

    let ciphertext = cipher
        .encrypt(&nonce, key_bytes)
        .map_err(|e| anyhow!("Encryption failed: {e}"))?;

    // 3. Assemble: magic(4) + salt(16) + nonce(12) + ciphertext(N+16)
    let mut output = Vec::with_capacity(HEADER_SIZE + ciphertext.len());
    output.extend_from_slice(ENCRYPTED_KEY_MAGIC);
    output.extend_from_slice(&salt);
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);

    fs::write(path, &output)?;
    set_permissions(path)?;

    // Zero out the derived key using volatile writes that the compiler
    // cannot optimize away (unlike `fill(0)` which may be elided).
    aes_key.zeroize();
    Ok(())
}

/// Decrypt an LKEF-format key blob using the given passphrase.
///
/// This is the testable core of encrypted key loading — it takes raw file
/// bytes and a passphrase directly, bypassing the interactive prompt / env var
/// resolution.  Callers that need to resolve the passphrase from the environment
/// or user input should use [`load_key`] instead.
///
/// # Format
/// `data` is expected to be: `LKEF(4) || salt(16) || nonce(12) || ciphertext(N+16)`
pub(crate) fn load_encrypted_key_data(data: &[u8], passphrase: &str) -> Result<Vec<u8>> {
    if data.len() < HEADER_SIZE + 1 {
        return Err(anyhow!("Invalid encrypted key file: too short"));
    }

    let salt = &data[4..20]; // after magic
    let nonce_bytes = &data[20..32];
    let ciphertext = &data[32..];

    // Derive key
    let mut aes_key = [0u8; 32];
    Argon2::default()
        .hash_password_into(passphrase.as_bytes(), salt, &mut aes_key)
        .map_err(|e| anyhow!("Argon2id KDF failed: {e}"))?;

    let cipher = Aes256Gcm::new_from_slice(&aes_key).map_err(|_| anyhow!("AES init failed"))?;
    let nonce = Nonce::from_slice(nonce_bytes);

    let result = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| anyhow!("Decryption failed: incorrect passphrase or corrupted key file"))?;

    aes_key.zeroize();
    Ok(result)
}

// ─────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────

fn set_permissions(_path: &std::path::Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(_path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_save_and_load_raw() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("test_key");
        let key_data = b"this-is-a-test-key-32-bytes!!";

        save_key(&path, key_data, None)?;
        assert!(path.exists(), "Raw key file must exist");

        let loaded = load_key(&path)?;
        assert_eq!(loaded, key_data, "Raw key data must match after load");
        assert!(!is_encrypted(&path), "Raw key must not be marked encrypted");
        Ok(())
    }

    #[test]
    fn test_save_and_load_encrypted_with_passphrase() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("test_key_enc");
        let key_data = b"test-key-data-for-encryption";
        let passphrase = "test-passphrase-123";

        save_key(&path, key_data, Some(passphrase))?;
        assert!(path.exists(), "Encrypted key file must exist");
        assert!(is_encrypted(&path), "Key must be marked encrypted");

        let file_bytes = std::fs::read(&path)?;
        let loaded = load_encrypted_key_data(&file_bytes, passphrase)?;
        assert_eq!(
            loaded, key_data,
            "Encrypted key data must match after load/decrypt"
        );
        Ok(())
    }

    #[test]
    fn test_encrypted_wrong_passphrase_rejected() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("test_key_wrong_pw");
        let key_data = b"test-key-data-for-wrong-passphrase";
        let correct_pw = "correct-passphrase";
        let wrong_pw = "wrong-passphrase";

        save_key(&path, key_data, Some(correct_pw))?;

        let file_bytes = std::fs::read(&path)?;
        match load_encrypted_key_data(&file_bytes, wrong_pw) {
            Err(e) => {
                let err_msg = e.to_string();
                assert!(
                    err_msg.contains("incorrect passphrase"),
                    "Error must mention incorrect passphrase, got: {}",
                    err_msg
                );
            }
            Ok(_) => panic!("Wrong passphrase must fail decryption"),
        }
        Ok(())
    }

    #[test]
    fn test_encrypted_file_format() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("test_key_format");
        let key_data = b"test-key-data-for-format-check";
        let passphrase = "correct-passphrase";

        save_key(&path, key_data, Some(passphrase))?;

        let file_bytes = std::fs::read(&path)?;
        assert!(
            file_bytes.starts_with(b"LKEF"),
            "File must start with LKEF magic"
        );
        let expected_len = 4 + 16 + 12 + key_data.len() + 16;
        assert_eq!(
            file_bytes.len(),
            expected_len,
            "File must have correct size: expected {}, got {}",
            expected_len,
            file_bytes.len()
        );
        Ok(())
    }

    #[test]
    fn test_empty_passphrase_treated_as_raw() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("test_key_empty_pw");
        let key_data = b"test-key-data";

        save_key(&path, key_data, Some(""))?;
        assert!(!is_encrypted(&path), "Empty passphrase should not encrypt");

        let loaded = load_key(&path)?;
        assert_eq!(loaded, key_data);
        Ok(())
    }

    #[test]
    fn test_encrypted_corrupted_file_rejected() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("test_key_corrupt");
        let key_data = b"test-key-data";
        let passphrase = "test-passphrase";

        save_key(&path, key_data, Some(passphrase))?;

        let mut file_bytes = std::fs::read(&path)?;
        let corrupt_pos = file_bytes.len() - 10;
        file_bytes[corrupt_pos] ^= 0xFF;

        let result = load_encrypted_key_data(&file_bytes, passphrase);
        assert!(
            result.is_err(),
            "Corrupted encrypted key must fail decryption"
        );
        Ok(())
    }

    #[test]
    fn test_encrypted_too_short_data_rejected() -> Result<()> {
        match load_encrypted_key_data(b"LKEF", "passphrase") {
            Err(e) => {
                assert!(
                    e.to_string().contains("too short"),
                    "Error must mention too short, got: {}",
                    e
                );
            }
            Ok(_) => panic!("Too short encrypted data must be rejected"),
        }
        Ok(())
    }
}
