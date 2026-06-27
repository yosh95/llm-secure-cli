use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use anyhow::{Result, anyhow};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

// FIPS 203: ML-KEM-512, ML-KEM-768, ML-KEM-1024
use fips203::ml_kem_512;
use fips203::ml_kem_768;
use fips203::ml_kem_1024;
use fips203::traits::{Decaps, Encaps, KeyGen as KemKeyGen, SerDes as KemSerDes};

// FIPS 204: ML-DSA-44, ML-DSA-65, ML-DSA-87
use fips204::ml_dsa_44;
use fips204::ml_dsa_65;
use fips204::ml_dsa_87;
use fips204::traits::{KeyGen as DsaKeyGen, SerDes as DsaSerDes, Signer, Verifier};

/// Post-quantum signature algorithm variant (FIPS 204 ML-DSA).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PQCVariant {
    /// ML-DSA-44 (FIPS 204, NIST Level 2): lowest security, fastest, smallest keys.
    MLDSA44,
    /// ML-DSA-65 (FIPS 204, NIST Level 3): balanced security.
    MLDSA65,
    /// ML-DSA-87 (FIPS 204, NIST Level 5): highest security, slowest, largest keys.
    MLDSA87,
}

/// Post-quantum KEM algorithm variant (FIPS 203 ML-KEM).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KEMVariant {
    /// ML-KEM-512 (FIPS 203, NIST Level 1): lowest security, fastest, smallest keys.
    MLKEM512,
    /// ML-KEM-768 (FIPS 203, NIST Level 3): balanced security.
    MLKEM768,
    /// ML-KEM-1024 (FIPS 203, NIST Level 5): highest security, slowest, largest keys.
    MLKEM1024,
}

impl PQCVariant {
    #[must_use]
    pub fn to_str(&self) -> &'static str {
        match self {
            PQCVariant::MLDSA44 => "ML-DSA-44",
            PQCVariant::MLDSA65 => "ML-DSA-65",
            PQCVariant::MLDSA87 => "ML-DSA-87",
        }
    }

    /// Returns the key file suffix used for this variant.
    #[must_use]
    pub fn key_suffix(&self) -> &'static str {
        match self {
            PQCVariant::MLDSA44 => "mldsa44",
            PQCVariant::MLDSA65 => "mldsa65",
            PQCVariant::MLDSA87 => "mldsa87",
        }
    }

    /// Returns the key file name (without path) for this variant.
    #[must_use]
    pub fn key_filename(&self) -> String {
        format!("id_{}", self.key_suffix())
    }

    /// Returns the public key filename (without path) for this variant.
    #[must_use]
    pub fn pub_key_filename(&self) -> String {
        format!("id_{}.pub", self.key_suffix())
    }
}

impl KEMVariant {
    #[must_use]
    pub fn to_str(&self) -> &'static str {
        match self {
            KEMVariant::MLKEM512 => "ML-KEM-512",
            KEMVariant::MLKEM768 => "ML-KEM-768",
            KEMVariant::MLKEM1024 => "ML-KEM-1024",
        }
    }

    /// Returns the key file suffix used for this variant.
    #[must_use]
    pub fn key_suffix(&self) -> &'static str {
        match self {
            KEMVariant::MLKEM512 => "kem512",
            KEMVariant::MLKEM768 => "kem768",
            KEMVariant::MLKEM1024 => "kem1024",
        }
    }

    /// Returns the key file name (without path) for this variant.
    #[must_use]
    pub fn key_filename(&self) -> String {
        format!("id_{}", self.key_suffix())
    }

    /// Returns the public key filename (without path) for this variant.
    #[must_use]
    pub fn pub_key_filename(&self) -> String {
        format!("id_{}.pub", self.key_suffix())
    }
}

impl FromStr for PQCVariant {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().replace('_', "-").as_str() {
            "ML-DSA-44" | "MLDSA44" => Ok(PQCVariant::MLDSA44),
            "ML-DSA-65" | "MLDSA65" => Ok(PQCVariant::MLDSA65),
            "ML-DSA-87" | "MLDSA87" => Ok(PQCVariant::MLDSA87),
            _ => Err(anyhow!(
                "Unknown PQC signature variant: {s}. Supported: ML-DSA-44, ML-DSA-65, ML-DSA-87"
            )),
        }
    }
}

impl FromStr for KEMVariant {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().replace('_', "-").as_str() {
            "ML-KEM-512" | "MLKEM512" => Ok(KEMVariant::MLKEM512),
            "ML-KEM-768" | "MLKEM768" => Ok(KEMVariant::MLKEM768),
            "ML-KEM-1024" | "MLKEM1024" => Ok(KEMVariant::MLKEM1024),
            _ => Err(anyhow!(
                "Unknown PQC KEM variant: {s}. Supported: ML-KEM-512, ML-KEM-768, ML-KEM-1024"
            )),
        }
    }
}

pub type MldsaVariant = PQCVariant;
pub type MlkemVariant = KEMVariant;

/// Default PQC signature variant: ML-DSA-44 (lowest security, fastest).
pub const DEFAULT_PQC_VARIANT: PQCVariant = PQCVariant::MLDSA44;

/// Default PQC KEM variant: ML-KEM-512 (lowest security, fastest).
pub const DEFAULT_KEM_VARIANT: KEMVariant = KEMVariant::MLKEM512;

pub struct PqcProvider;

impl PqcProvider {
    pub fn generate_keypair(variant: PQCVariant) -> Result<(Vec<u8>, Vec<u8>)> {
        match variant {
            PQCVariant::MLDSA44 => {
                let (pk, sk) = ml_dsa_44::KG::try_keygen()
                    .map_err(|e| anyhow!("ML-DSA-44 keygen failed: {e}"))?;
                Ok((pk.into_bytes().to_vec(), sk.into_bytes().to_vec()))
            }
            PQCVariant::MLDSA65 => {
                let (pk, sk) = ml_dsa_65::KG::try_keygen()
                    .map_err(|e| anyhow!("ML-DSA-65 keygen failed: {e}"))?;
                Ok((pk.into_bytes().to_vec(), sk.into_bytes().to_vec()))
            }
            PQCVariant::MLDSA87 => {
                let (pk, sk) = ml_dsa_87::KG::try_keygen()
                    .map_err(|e| anyhow!("ML-DSA-87 keygen failed: {e}"))?;
                Ok((pk.into_bytes().to_vec(), sk.into_bytes().to_vec()))
            }
        }
    }

    pub fn generate_kem_keypair(variant: KEMVariant) -> Result<(Vec<u8>, Vec<u8>)> {
        match variant {
            KEMVariant::MLKEM512 => {
                let (pk, sk) = ml_kem_512::KG::try_keygen()
                    .map_err(|e| anyhow!("ML-KEM-512 keygen failed: {e}"))?;
                Ok((pk.into_bytes().to_vec(), sk.into_bytes().to_vec()))
            }
            KEMVariant::MLKEM768 => {
                let (pk, sk) = ml_kem_768::KG::try_keygen()
                    .map_err(|e| anyhow!("ML-KEM-768 keygen failed: {e}"))?;
                Ok((pk.into_bytes().to_vec(), sk.into_bytes().to_vec()))
            }
            KEMVariant::MLKEM1024 => {
                let (pk, sk) = ml_kem_1024::KG::try_keygen()
                    .map_err(|e| anyhow!("ML-KEM-1024 keygen failed: {e}"))?;
                Ok((pk.into_bytes().to_vec(), sk.into_bytes().to_vec()))
            }
        }
    }

    pub fn sign(variant: PQCVariant, sk_bytes: &[u8], message: &[u8]) -> Result<Vec<u8>> {
        match variant {
            PQCVariant::MLDSA44 => {
                let sk_arr: [u8; ml_dsa_44::SK_LEN] = sk_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-DSA-44 secret key length"))?;
                let sk = ml_dsa_44::PrivateKey::try_from_bytes(sk_arr)
                    .map_err(|e| anyhow!("Invalid ML-DSA-44 sk: {e}"))?;
                let sig = sk
                    .try_sign(message, &[])
                    .map_err(|e| anyhow!("ML-DSA-44 sign failed: {e}"))?;
                Ok(sig.to_vec())
            }
            PQCVariant::MLDSA65 => {
                let sk_arr: [u8; ml_dsa_65::SK_LEN] = sk_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-DSA-65 secret key length"))?;
                let sk = ml_dsa_65::PrivateKey::try_from_bytes(sk_arr)
                    .map_err(|e| anyhow!("Invalid ML-DSA-65 sk: {e}"))?;
                let sig = sk
                    .try_sign(message, &[])
                    .map_err(|e| anyhow!("ML-DSA-65 sign failed: {e}"))?;
                Ok(sig.to_vec())
            }
            PQCVariant::MLDSA87 => {
                let sk_arr: [u8; ml_dsa_87::SK_LEN] = sk_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-DSA-87 secret key length"))?;
                let sk = ml_dsa_87::PrivateKey::try_from_bytes(sk_arr)
                    .map_err(|e| anyhow!("Invalid ML-DSA-87 sk: {e}"))?;
                let sig = sk
                    .try_sign(message, &[])
                    .map_err(|e| anyhow!("ML-DSA-87 sign failed: {e}"))?;
                Ok(sig.to_vec())
            }
        }
    }

    pub fn verify(
        variant: PQCVariant,
        pk_bytes: &[u8],
        message: &[u8],
        sig_bytes: &[u8],
    ) -> Result<()> {
        match variant {
            PQCVariant::MLDSA44 => {
                let pk_arr: [u8; ml_dsa_44::PK_LEN] = pk_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-DSA-44 public key length"))?;
                let sig_arr: [u8; ml_dsa_44::SIG_LEN] = sig_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-DSA-44 signature length"))?;
                let pk = ml_dsa_44::PublicKey::try_from_bytes(pk_arr)
                    .map_err(|e| anyhow!("Invalid ML-DSA-44 pk: {e}"))?;
                if pk.verify(message, &sig_arr, &[]) {
                    Ok(())
                } else {
                    Err(anyhow!("PQC Verification failed (ML-DSA-44)"))
                }
            }
            PQCVariant::MLDSA65 => {
                let pk_arr: [u8; ml_dsa_65::PK_LEN] = pk_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-DSA-65 public key length"))?;
                let sig_arr: [u8; ml_dsa_65::SIG_LEN] = sig_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-DSA-65 signature length"))?;
                let pk = ml_dsa_65::PublicKey::try_from_bytes(pk_arr)
                    .map_err(|e| anyhow!("Invalid ML-DSA-65 pk: {e}"))?;
                if pk.verify(message, &sig_arr, &[]) {
                    Ok(())
                } else {
                    Err(anyhow!("PQC Verification failed (ML-DSA-65)"))
                }
            }
            PQCVariant::MLDSA87 => {
                let pk_arr: [u8; ml_dsa_87::PK_LEN] = pk_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-DSA-87 public key length"))?;
                let sig_arr: [u8; ml_dsa_87::SIG_LEN] = sig_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-DSA-87 signature length"))?;
                let pk = ml_dsa_87::PublicKey::try_from_bytes(pk_arr)
                    .map_err(|e| anyhow!("Invalid ML-DSA-87 pk: {e}"))?;
                if pk.verify(message, &sig_arr, &[]) {
                    Ok(())
                } else {
                    Err(anyhow!("PQC Verification failed (ML-DSA-87)"))
                }
            }
        }
    }

    pub fn sign_mldsa(message: &[u8], sk_bytes: &[u8], variant: PQCVariant) -> Result<Vec<u8>> {
        Self::sign(variant, sk_bytes, message)
    }

    #[must_use]
    pub fn verify_mldsa(
        message: &[u8],
        sig_bytes: &[u8],
        pk_bytes: &[u8],
        variant: PQCVariant,
    ) -> bool {
        Self::verify(variant, pk_bytes, message, sig_bytes).is_ok()
    }

    pub fn encapsulate(variant: KEMVariant, pk_bytes: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
        match variant {
            KEMVariant::MLKEM512 => {
                let pk_arr: [u8; ml_kem_512::EK_LEN] = pk_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-KEM-512 public key length"))?;
                let ek = ml_kem_512::EncapsKey::try_from_bytes(pk_arr)
                    .map_err(|e| anyhow!("Invalid ML-KEM-512 pk: {e}"))?;
                let (ss, ct) = ek
                    .try_encaps()
                    .map_err(|e| anyhow!("ML-KEM-512 encapsulate failed: {e}"))?;
                Ok((ss.into_bytes().to_vec(), ct.into_bytes().to_vec()))
            }
            KEMVariant::MLKEM768 => {
                let pk_arr: [u8; ml_kem_768::EK_LEN] = pk_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-KEM-768 public key length"))?;
                let ek = ml_kem_768::EncapsKey::try_from_bytes(pk_arr)
                    .map_err(|e| anyhow!("Invalid ML-KEM-768 pk: {e}"))?;
                let (ss, ct) = ek
                    .try_encaps()
                    .map_err(|e| anyhow!("ML-KEM-768 encapsulate failed: {e}"))?;
                Ok((ss.into_bytes().to_vec(), ct.into_bytes().to_vec()))
            }
            KEMVariant::MLKEM1024 => {
                let pk_arr: [u8; ml_kem_1024::EK_LEN] = pk_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-KEM-1024 public key length"))?;
                let ek = ml_kem_1024::EncapsKey::try_from_bytes(pk_arr)
                    .map_err(|e| anyhow!("Invalid ML-KEM-1024 pk: {e}"))?;
                let (ss, ct) = ek
                    .try_encaps()
                    .map_err(|e| anyhow!("ML-KEM-1024 encapsulate failed: {e}"))?;
                Ok((ss.into_bytes().to_vec(), ct.into_bytes().to_vec()))
            }
        }
    }

    pub fn decapsulate(variant: KEMVariant, ct_bytes: &[u8], sk_bytes: &[u8]) -> Result<Vec<u8>> {
        match variant {
            KEMVariant::MLKEM512 => {
                let dk_arr: [u8; ml_kem_512::DK_LEN] = sk_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-KEM-512 secret key length"))?;
                let ct_arr: [u8; ml_kem_512::CT_LEN] = ct_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-KEM-512 ciphertext length"))?;
                let dk = ml_kem_512::DecapsKey::try_from_bytes(dk_arr)
                    .map_err(|e| anyhow!("Invalid ML-KEM-512 dk: {e}"))?;
                let ct = ml_kem_512::CipherText::try_from_bytes(ct_arr)
                    .map_err(|e| anyhow!("Invalid ML-KEM-512 ct: {e}"))?;
                let ss = dk
                    .try_decaps(&ct)
                    .map_err(|e| anyhow!("ML-KEM-512 decapsulate failed: {e}"))?;
                Ok(ss.into_bytes().to_vec())
            }
            KEMVariant::MLKEM768 => {
                let dk_arr: [u8; ml_kem_768::DK_LEN] = sk_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-KEM-768 secret key length"))?;
                let ct_arr: [u8; ml_kem_768::CT_LEN] = ct_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-KEM-768 ciphertext length"))?;
                let dk = ml_kem_768::DecapsKey::try_from_bytes(dk_arr)
                    .map_err(|e| anyhow!("Invalid ML-KEM-768 dk: {e}"))?;
                let ct = ml_kem_768::CipherText::try_from_bytes(ct_arr)
                    .map_err(|e| anyhow!("Invalid ML-KEM-768 ct: {e}"))?;
                let ss = dk
                    .try_decaps(&ct)
                    .map_err(|e| anyhow!("ML-KEM-768 decapsulate failed: {e}"))?;
                Ok(ss.into_bytes().to_vec())
            }
            KEMVariant::MLKEM1024 => {
                let dk_arr: [u8; ml_kem_1024::DK_LEN] = sk_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-KEM-1024 secret key length"))?;
                let ct_arr: [u8; ml_kem_1024::CT_LEN] = ct_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-KEM-1024 ciphertext length"))?;
                let dk = ml_kem_1024::DecapsKey::try_from_bytes(dk_arr)
                    .map_err(|e| anyhow!("Invalid ML-KEM-1024 dk: {e}"))?;
                let ct = ml_kem_1024::CipherText::try_from_bytes(ct_arr)
                    .map_err(|e| anyhow!("Invalid ML-KEM-1024 ct: {e}"))?;
                let ss = dk
                    .try_decaps(&ct)
                    .map_err(|e| anyhow!("ML-KEM-1024 decapsulate failed: {e}"))?;
                Ok(ss.into_bytes().to_vec())
            }
        }
    }
}

/// Encrypted payload using ML-KEM + AES-256-GCM hybrid encryption.
///
/// # Field layout
/// The AES-256-GCM authentication tag is stored in a **separate field** (`tag`)
/// rather than being appended to the ciphertext (the more common `ciphertext ||
/// tag` convention used by most AEAD libraries).  This split is intentional:
///
/// - **Structural clarity**: each component (ciphertext, nonce, tag) is
///   independently addressable, simplifying serialization and debugging.
/// - **Interoperability note**: consumers that expect a standard concatenated
///   AEAD payload must reconstruct it as `[aes_ct, tag].concat()`.
/// - **Security**: the tag is always verified via `Aes256Gcm::decrypt()` which
///   rejoins them internally; the split does not weaken integrity guarantees.
#[derive(Serialize, Deserialize)]
pub struct EncryptedPacket {
    pub kem_ct: Vec<u8>,
    pub aes_ct: Vec<u8>,
    pub nonce: Vec<u8>,
    pub tag: Vec<u8>,
    pub algo: String,
}

pub struct SecureStorage;

impl SecureStorage {
    /// Encrypt data using ML-KEM + AES-256-GCM hybrid encryption.
    /// Uses the default KEM variant (ML-KEM-512).
    pub fn encrypt(data: &[u8], recipient_public_key: &[u8]) -> Result<EncryptedPacket> {
        Self::encrypt_with_variant(data, recipient_public_key, DEFAULT_KEM_VARIANT)
    }

    pub fn encrypt_with_variant(
        data: &[u8],
        recipient_public_key: &[u8],
        variant: KEMVariant,
    ) -> Result<EncryptedPacket> {
        let (shared_secret, kem_ct) = PqcProvider::encapsulate(variant, recipient_public_key)?;
        let key = &shared_secret[..32];
        let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| anyhow!("AES init failed"))?;
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from(nonce_bytes);
        let ciphertext_with_tag = cipher
            .encrypt(&nonce, data)
            .map_err(|e| anyhow!("Enc failed: {e}"))?;
        let (aes_ct, tag) = ciphertext_with_tag.split_at(ciphertext_with_tag.len() - 16);
        Ok(EncryptedPacket {
            kem_ct,
            aes_ct: aes_ct.to_vec(),
            nonce: nonce_bytes.to_vec(),
            tag: tag.to_vec(),
            algo: format!("{}/AES-256-GCM", variant.to_str()),
        })
    }

    /// Decrypt a packet. The variant is determined from the `algo` field.
    pub fn decrypt(packet: &EncryptedPacket, private_key: &[u8]) -> Result<Vec<u8>> {
        // Determine the KEM variant from the algo field
        let variant = if packet.algo.starts_with("ML-KEM-512") {
            KEMVariant::MLKEM512
        } else if packet.algo.starts_with("ML-KEM-768") {
            KEMVariant::MLKEM768
        } else {
            KEMVariant::MLKEM1024
        };
        let ss = PqcProvider::decapsulate(variant, &packet.kem_ct, private_key)?;
        let key = &ss[..32];
        let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| anyhow!("AES init failed"))?;
        let nonce = Nonce::from_slice(&packet.nonce);
        let mut encrypted = packet.aes_ct.clone();
        encrypted.extend_from_slice(&packet.tag);
        cipher
            .decrypt(nonce, encrypted.as_slice())
            .map_err(|e| anyhow!("Dec failed: {e}"))
    }
}

/// Returns the configured PQC signature variant.
///
/// Reads from `config.pqc.signature_variant` to determine the level.
/// Falls back to `DEFAULT_PQC_VARIANT` (ML-DSA-44) if config is missing or invalid.
#[must_use]
pub fn get_signature_variant(config: &crate::config::models::AppConfig) -> PQCVariant {
    let variant_str = &config.pqc.signature_variant;
    PQCVariant::from_str(variant_str).unwrap_or(DEFAULT_PQC_VARIANT)
}

/// Returns the configured PQC KEM variant.
///
/// Reads from `config.pqc.kem_variant` to determine the level.
/// Falls back to `DEFAULT_KEM_VARIANT` (ML-KEM-512) if config is missing or invalid.
#[must_use]
pub fn get_kem_variant(config: &crate::config::models::AppConfig) -> KEMVariant {
    let variant_str = &config.pqc.kem_variant;
    KEMVariant::from_str(variant_str).unwrap_or(DEFAULT_KEM_VARIANT)
}

pub struct ResponseSigner;
impl ResponseSigner {
    pub fn sign_response(
        text: &str,
        id: &str,
        sk: &[u8],
        v: PQCVariant,
    ) -> Result<serde_json::Value> {
        let msg = format!("{id}:{text}");
        let sig = PqcProvider::sign(v, sk, msg.as_bytes())?;
        Ok(serde_json::json!({
            "result": text,
            "verification_id": id,
            "pqc_signature": base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, sig),
            "algorithm": v.to_str()
        }))
    }
}
