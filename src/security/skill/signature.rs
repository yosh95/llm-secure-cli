use crate::security::skill::types::SkillSignatureStatus;
use std::path::Path;

/// Verifies the COSE signature of a SKILL.md file, if present.
///
/// Looks for `SKILL.md.sig` alongside `SKILL.md`. If found, verifies
/// the signature using the project's Ed25519/PQC verification pipeline.
/// If no signature file is found, returns `Unsigned`.
#[must_use]
pub fn verify_skill_signature(dir: &Path) -> SkillSignatureStatus {
    let skill_md = match find_skill_md(dir) {
        Ok(p) => p,
        Err(_) => return SkillSignatureStatus::Unsigned,
    };

    let sig_path = skill_md.with_extension("md.sig");

    if !sig_path.exists() {
        // Also try SKILL.md.sig in the same directory
        let alt_sig = skill_md.parent().map(|p| p.join("SKILL.md.sig"));
        if let Some(ref alt) = alt_sig
            && alt.exists()
        {
            return verify_skill_signature_impl(&skill_md, alt);
        }
        return SkillSignatureStatus::Unsigned;
    }

    verify_skill_signature_impl(&skill_md, &sig_path)
}

fn find_skill_md(dir: &Path) -> Result<std::path::PathBuf, ()> {
    if !dir.is_dir() {
        return Err(());
    }

    let exact = dir.join("SKILL.md");
    if exact.exists() {
        return Ok(exact);
    }

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy().to_lowercase();
            if name_str == "skill.md" {
                return Ok(entry.path());
            }
        }
    }

    Err(())
}

fn verify_skill_signature_impl(skill_md: &Path, sig_path: &Path) -> SkillSignatureStatus {
    let sig_data = match std::fs::read(sig_path) {
        Ok(d) => d,
        Err(e) => {
            return SkillSignatureStatus::VerificationFailed(format!(
                "Cannot read signature file: {e}"
            ));
        }
    };

    let skill_content = match std::fs::read(skill_md) {
        Ok(d) => d,
        Err(e) => {
            return SkillSignatureStatus::VerificationFailed(format!("Cannot read SKILL.md: {e}"));
        }
    };

    // Strategy 1: Try as a COSE hybrid token (Tag 98)
    if let Ok(pubkey) = crate::security::identity::IdentityManager::get_classical_public_key()
        && let Some(claims) = crate::security::pqc_cose::HybridSigner::verify_hybrid_token(
            &sig_data,
            &pubkey,
            |variant| {
                crate::security::identity::IdentityManager::get_pqc_public_key(variant)
                    .unwrap_or_default()
            },
        )
    {
        let publisher = claims
            .get("sub")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        return SkillSignatureStatus::Verified {
            publisher,
            algorithm: "Ed25519/ML-DSA".to_string(),
        };
    }

    // Strategy 2: Try as a raw Ed25519 signature (64 bytes)
    match verify_raw_ed25519(&skill_content, &sig_data) {
        Ok(pubkey_short) => SkillSignatureStatus::Verified {
            publisher: format!("ed25519:{pubkey_short}"),
            algorithm: "Ed25519".to_string(),
        },
        Err(_) => SkillSignatureStatus::VerificationFailed(
            "Signature verification failed: not a valid COSE token or Ed25519 signature"
                .to_string(),
        ),
    }
}

/// Fallback: verify a raw Ed25519 signature (64 bytes) against the
/// project's identity public key.
fn verify_raw_ed25519(
    _content: &[u8],
    _sig_bytes: &[u8],
) -> Result<String, Box<dyn std::error::Error>> {
    use crate::security::identity::IdentityManager;

    let pk = IdentityManager::get_classical_public_key()
        .map_err(|e| format!("Cannot load Ed25519 public key: {e}"))?;

    // Use the dalek API directly
    let sig = ed25519_dalek::Signature::from_slice(_sig_bytes)
        .map_err(|e| format!("Invalid Ed25519 signature bytes: {e}"))?;

    let pk_array: [u8; 32] = pk
        .try_into()
        .map_err(|_| "Ed25519 public key must be exactly 32 bytes")?;
    let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(&pk_array)
        .map_err(|e| format!("Invalid Ed25519 public key: {e}"))?;

    use ed25519_dalek::Verifier;
    verifying_key
        .verify(_content, &sig)
        .map_err(|e| format!("Ed25519 verification failed: {e}"))?;

    // Return a short identifier for the public key
    let hex_pk: String = pk_array.iter().map(|b| format!("{b:02x}")).collect();
    let short = if hex_pk.len() > 16 {
        format!("{}...", &hex_pk[..16])
    } else {
        hex_pk
    };

    Ok(short)
}
