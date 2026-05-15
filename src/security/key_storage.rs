use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use anyhow::{Result, anyhow};
use argon2::Argon2;
use rand::RngCore;
use std::fs;
use std::io::{IsTerminal, stdin};
use std::path::PathBuf;
use std::sync::Mutex;

/// Header magic bytes to identify encrypted key files.
const ENCRYPTED_KEY_MAGIC: &[u8; 4] = b"LKEF";
const HEADER_SIZE: usize = 4 + 16 + 12; // magic(4) + salt(16) + nonce(12) = 32

/// Thread-local cache for passphrase during a session.
/// Set once on first key access, reused for all subsequent key reads.
static PASSPHRASE_CACHE: Mutex<Option<String>> = Mutex::new(None);

// ─────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────

/// Persist private key material to disk, optionally encrypted with a passphrase.
///
/// If `passphrase` is `Some` and non-empty, the key is encrypted with AES-256-GCM
/// using an Argon2id-derived key. If `passphrase` is `None` or empty, the key is
/// stored as raw bytes (backwards compatible).
pub fn save_key(path: &PathBuf, key_bytes: &[u8], passphrase: Option<&str>) -> Result<()> {
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
pub fn load_key(path: &PathBuf) -> Result<Vec<u8>> {
    if !path.exists() {
        return Err(anyhow!("Key file not found: {:?}", path));
    }

    let mut file = fs::File::open(path)?;
    let mut magic = [0u8; 4];
    use std::io::Read;
    // If the file is too short or read fails, treat as raw (backward compat).
    if file.read_exact(&mut magic).is_err() || &magic != ENCRYPTED_KEY_MAGIC {
        // Fall back to reading the whole file as raw key bytes.
        return Ok(fs::read(path)?);
    }

    // Encrypted path
    let mut data = Vec::new();
    data.extend_from_slice(&magic);
    file.read_to_end(&mut data)?;
    let pw = read_passphrase_or_cached(false)?; // false = not optional for encrypted keys
    load_encrypted_inner(&data, &pw)
}

/// Purge the passphrase cache (called on session end).
pub fn purge_passphrase_cache() {
    if let Ok(mut cache) = PASSPHRASE_CACHE.lock()
        && let Some(pw) = cache.take()
    {
        // Zero the string before dropping
        let mut v = pw.into_bytes();
        v.fill(0);
    }
}

/// Returns true if the key file is encrypted (starts with LKEF magic).
pub fn is_encrypted(path: &PathBuf) -> bool {
    path.exists()
        && fs::read(path)
            .map(|data| data.starts_with(ENCRYPTED_KEY_MAGIC))
            .unwrap_or(false)
}

/// Prompts the user for an *optional* passphrase (empty = no encryption).
/// Returns `Ok(Some(pw))` for a non-empty passphrase, `Ok(None)` if empty.
/// In non-interactive mode, checks `LLM_CLI_KEY_PASSPHRASE` env var.
pub fn read_optional_passphrase() -> Result<Option<String>> {
    let is_atty = stdin().is_terminal();

    if is_atty {
        read_optional_passphrase_interactive()
    } else {
        // Non-interactive: check env var
        match std::env::var("LLM_CLI_KEY_PASSPHRASE") {
            Ok(pw) if !pw.trim().is_empty() => Ok(Some(pw.trim().to_string())),
            _ => {
                // Also check *_FILE variant (Docker secrets pattern)
                if let Ok(path) = std::env::var("LLM_CLI_KEY_PASSPHRASE_FILE")
                    && let Ok(content) = fs::read_to_string(&path)
                {
                    let pw = content.trim().to_string();
                    if !pw.is_empty() {
                        return Ok(Some(pw));
                    }
                }
                Ok(None) // Default: no passphrase
            }
        }
    }
}

// ─────────────────────────────────────────────
// Internal: interactive passphrase prompts
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

fn read_passphrase_or_cached(is_optional: bool) -> Result<String> {
    // 1. Check cache
    {
        let cache = PASSPHRASE_CACHE
            .lock()
            .map_err(|_| anyhow!("Cache lock poisoned"))?;
        if let Some(ref pw) = *cache {
            return Ok(pw.clone());
        }
    }

    // 2. Not cached — read from appropriate source
    let is_atty = stdin().is_terminal();

    let pw = if is_atty {
        if is_optional {
            match read_optional_passphrase_interactive()? {
                Some(pw) => pw,
                None => return Err(anyhow!("Passphrase required for encrypted keys")),
            }
        } else {
            read_passphrase_interactive()?
        }
    } else {
        // Non-interactive: env var is required for encrypted keys
        let pw = std::env::var("LLM_CLI_KEY_PASSPHRASE")
            .or_else(|_| {
                std::env::var("LLM_CLI_KEY_PASSPHRASE_FILE").and_then(|path| {
                    fs::read_to_string(path).map_err(|_| std::env::VarError::NotPresent)
                })
            })
            .map(|s| s.trim().to_string())
            .map_err(|_| {
                anyhow!(
                    "Encrypted PQC keys require a passphrase.\n\
                 For interactive use, run from a terminal.\n\
                 For non-interactive use, set LLM_CLI_KEY_PASSPHRASE environment variable."
                )
            })?;
        if pw.is_empty() {
            return Err(anyhow!(
                "LLM_CLI_KEY_PASSPHRASE is empty — passphrase required"
            ));
        }
        pw
    };

    // 3. Cache for session
    {
        let mut cache = PASSPHRASE_CACHE
            .lock()
            .map_err(|_| anyhow!("Cache lock poisoned"))?;
        *cache = Some(pw.clone());
    }

    Ok(pw)
}

// ─────────────────────────────────────────────
// Internal: encryption / decryption
// ─────────────────────────────────────────────

fn save_encrypted(path: &PathBuf, key_bytes: &[u8], passphrase: &str) -> Result<()> {
    // 1. Derive AES key via Argon2id
    let mut aes_key = [0u8; 32];
    let mut salt = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut salt);

    Argon2::default()
        .hash_password_into(passphrase.as_bytes(), &salt, &mut aes_key)
        .map_err(|e| anyhow!("Argon2id KDF failed: {}", e))?;

    // 2. AES-256-GCM encrypt
    let cipher = Aes256Gcm::new_from_slice(&aes_key).map_err(|_| anyhow!("AES init failed"))?;

    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from(nonce_bytes);

    let ciphertext = cipher
        .encrypt(&nonce, key_bytes)
        .map_err(|e| anyhow!("Encryption failed: {}", e))?;

    // 3. Assemble: magic(4) + salt(16) + nonce(12) + ciphertext(N+16)
    let mut output = Vec::with_capacity(HEADER_SIZE + ciphertext.len());
    output.extend_from_slice(ENCRYPTED_KEY_MAGIC);
    output.extend_from_slice(&salt);
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);

    fs::write(path, &output)?;
    set_permissions(path)?;

    // Zero out the derived key
    aes_key.fill(0);
    Ok(())
}

fn load_encrypted_inner(data: &[u8], passphrase: &str) -> Result<Vec<u8>> {
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
        .map_err(|e| anyhow!("Argon2id KDF failed: {}", e))?;

    let cipher = Aes256Gcm::new_from_slice(&aes_key).map_err(|_| anyhow!("AES init failed"))?;
    let nonce = Nonce::from_slice(nonce_bytes);

    let result = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| anyhow!("Decryption failed: incorrect passphrase or corrupted key file"))?;

    aes_key.fill(0);
    Ok(result)
}

// ─────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────

fn set_permissions(path: &PathBuf) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
