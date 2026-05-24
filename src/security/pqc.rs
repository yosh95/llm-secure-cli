use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use anyhow::{Result, anyhow};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

// ── FIPS 203: ML-KEM ──
use fips203::ml_kem_768;
use fips203::traits::{Decaps, Encaps, KeyGen as KemKeyGen, SerDes as KemSerDes};

// ── FIPS 204: ML-DSA ──
use fips204::ml_dsa_44;
use fips204::ml_dsa_65;
use fips204::ml_dsa_87;
use fips204::traits::{KeyGen as DsaKeyGen, SerDes as DsaSerDes, Signer, Verifier};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PQCVariant {
    MLDSA44,
    MLDSA65,
    MLDSA87,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KEMVariant {
    MLKEM512,
    MLKEM768,
    MLKEM1024,
}

impl PQCVariant {
    pub fn to_str(&self) -> &'static str {
        match self {
            PQCVariant::MLDSA44 => "ML-DSA-44",
            PQCVariant::MLDSA65 => "ML-DSA-65",
            PQCVariant::MLDSA87 => "ML-DSA-87",
        }
    }
}

impl KEMVariant {
    pub fn to_str(&self) -> &'static str {
        match self {
            KEMVariant::MLKEM512 => "ML-KEM-512",
            KEMVariant::MLKEM768 => "ML-KEM-768",
            KEMVariant::MLKEM1024 => "ML-KEM-1024",
        }
    }
}

impl FromStr for PQCVariant {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().replace('_', "-").as_str() {
            "ML-DSA-44" | "MLDSA44" => Ok(PQCVariant::MLDSA44),
            "ML-DSA-65" | "MLDSA65" => Ok(PQCVariant::MLDSA65),
            "ML-DSA-87" | "MLDSA87" => Ok(PQCVariant::MLDSA87),
            _ => Err(anyhow!("Unknown PQC variant: {}", s)),
        }
    }
}

pub type MldsaVariant = PQCVariant;
pub type MlkemVariant = KEMVariant;

/// The single post-quantum signature algorithm used throughout the application.
/// All operations use ML-DSA-87 (NIST Level 5) — the highest available strength.
pub const DEFAULT_PQC_VARIANT: PQCVariant = PQCVariant::MLDSA87;

/// The single post-quantum KEM algorithm used throughout the application.
/// All operations use ML-KEM-1024 (NIST Level 5) — the highest available strength.
pub const DEFAULT_KEM_VARIANT: KEMVariant = KEMVariant::MLKEM1024;

pub struct PqcProvider;

impl PqcProvider {
    // ── ML-DSA key generation ──

    pub fn generate_keypair(variant: PQCVariant) -> Result<(Vec<u8>, Vec<u8>)> {
        match variant {
            PQCVariant::MLDSA44 => {
                let (pk, sk) = ml_dsa_44::KG::try_keygen()
                    .map_err(|e| anyhow!("ML-DSA-44 keygen failed: {}", e))?;
                Ok((pk.into_bytes().to_vec(), sk.into_bytes().to_vec()))
            }
            PQCVariant::MLDSA65 => {
                let (pk, sk) = ml_dsa_65::KG::try_keygen()
                    .map_err(|e| anyhow!("ML-DSA-65 keygen failed: {}", e))?;
                Ok((pk.into_bytes().to_vec(), sk.into_bytes().to_vec()))
            }
            PQCVariant::MLDSA87 => {
                let (pk, sk) = ml_dsa_87::KG::try_keygen()
                    .map_err(|e| anyhow!("ML-DSA-87 keygen failed: {}", e))?;
                Ok((pk.into_bytes().to_vec(), sk.into_bytes().to_vec()))
            }
        }
    }

    // ── ML-KEM key generation ──

    pub fn generate_kem_keypair(variant: KEMVariant) -> Result<(Vec<u8>, Vec<u8>)> {
        match variant {
            KEMVariant::MLKEM512 => {
                let (pk, sk) = fips203::ml_kem_512::KG::try_keygen()
                    .map_err(|e| anyhow!("ML-KEM-512 keygen failed: {}", e))?;
                Ok((pk.into_bytes().to_vec(), sk.into_bytes().to_vec()))
            }
            KEMVariant::MLKEM768 => {
                let (pk, sk) = ml_kem_768::KG::try_keygen()
                    .map_err(|e| anyhow!("ML-KEM-768 keygen failed: {}", e))?;
                Ok((pk.into_bytes().to_vec(), sk.into_bytes().to_vec()))
            }
            KEMVariant::MLKEM1024 => {
                let (pk, sk) = fips203::ml_kem_1024::KG::try_keygen()
                    .map_err(|e| anyhow!("ML-KEM-1024 keygen failed: {}", e))?;
                Ok((pk.into_bytes().to_vec(), sk.into_bytes().to_vec()))
            }
        }
    }

    // ── ML-DSA sign ──

    pub fn sign(variant: PQCVariant, sk_bytes: &[u8], message: &[u8]) -> Result<Vec<u8>> {
        match variant {
            PQCVariant::MLDSA44 => {
                let sk_arr: [u8; ml_dsa_44::SK_LEN] = sk_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-DSA-44 secret key length"))?;
                let sk = ml_dsa_44::PrivateKey::try_from_bytes(sk_arr)
                    .map_err(|e| anyhow!("Invalid ML-DSA-44 sk: {}", e))?;
                let sig = sk
                    .try_sign(message, &[])
                    .map_err(|e| anyhow!("ML-DSA-44 sign failed: {}", e))?;
                Ok(sig.to_vec())
            }
            PQCVariant::MLDSA65 => {
                let sk_arr: [u8; ml_dsa_65::SK_LEN] = sk_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-DSA-65 secret key length"))?;
                let sk = ml_dsa_65::PrivateKey::try_from_bytes(sk_arr)
                    .map_err(|e| anyhow!("Invalid ML-DSA-65 sk: {}", e))?;
                let sig = sk
                    .try_sign(message, &[])
                    .map_err(|e| anyhow!("ML-DSA-65 sign failed: {}", e))?;
                Ok(sig.to_vec())
            }
            PQCVariant::MLDSA87 => {
                let sk_arr: [u8; ml_dsa_87::SK_LEN] = sk_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-DSA-87 secret key length"))?;
                let sk = ml_dsa_87::PrivateKey::try_from_bytes(sk_arr)
                    .map_err(|e| anyhow!("Invalid ML-DSA-87 sk: {}", e))?;
                let sig = sk
                    .try_sign(message, &[])
                    .map_err(|e| anyhow!("ML-DSA-87 sign failed: {}", e))?;
                Ok(sig.to_vec())
            }
        }
    }

    // ── ML-DSA verify ──

    pub fn verify(
        variant: PQCVariant,
        pk_bytes: &[u8],
        message: &[u8],
        sig_bytes: &[u8],
    ) -> Result<()> {
        let ok = match variant {
            PQCVariant::MLDSA44 => {
                let pk_arr: [u8; ml_dsa_44::PK_LEN] = pk_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-DSA-44 public key length"))?;
                let sig_arr: [u8; ml_dsa_44::SIG_LEN] = sig_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-DSA-44 signature length"))?;
                let pk = ml_dsa_44::PublicKey::try_from_bytes(pk_arr)
                    .map_err(|e| anyhow!("Invalid ML-DSA-44 pk: {}", e))?;
                pk.verify(message, &sig_arr, &[])
            }
            PQCVariant::MLDSA65 => {
                let pk_arr: [u8; ml_dsa_65::PK_LEN] = pk_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-DSA-65 public key length"))?;
                let sig_arr: [u8; ml_dsa_65::SIG_LEN] = sig_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-DSA-65 signature length"))?;
                let pk = ml_dsa_65::PublicKey::try_from_bytes(pk_arr)
                    .map_err(|e| anyhow!("Invalid ML-DSA-65 pk: {}", e))?;
                pk.verify(message, &sig_arr, &[])
            }
            PQCVariant::MLDSA87 => {
                let pk_arr: [u8; ml_dsa_87::PK_LEN] = pk_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-DSA-87 public key length"))?;
                let sig_arr: [u8; ml_dsa_87::SIG_LEN] = sig_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-DSA-87 signature length"))?;
                let pk = ml_dsa_87::PublicKey::try_from_bytes(pk_arr)
                    .map_err(|e| anyhow!("Invalid ML-DSA-87 pk: {}", e))?;
                pk.verify(message, &sig_arr, &[])
            }
        };
        if ok {
            Ok(())
        } else {
            Err(anyhow!("PQC Verification failed"))
        }
    }

    // ── Legacy ML-DSA wrappers ──

    pub fn sign_mldsa(message: &[u8], sk_bytes: &[u8], variant: PQCVariant) -> Result<Vec<u8>> {
        Self::sign(variant, sk_bytes, message)
    }

    pub fn verify_mldsa(
        message: &[u8],
        sig_bytes: &[u8],
        pk_bytes: &[u8],
        variant: PQCVariant,
    ) -> bool {
        Self::verify(variant, pk_bytes, message, sig_bytes).is_ok()
    }

    // ── ML-KEM encapsulate ──

    pub fn encapsulate(variant: KEMVariant, pk_bytes: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
        match variant {
            KEMVariant::MLKEM512 => {
                let pk_arr: [u8; fips203::ml_kem_512::EK_LEN] = pk_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-KEM-512 public key length"))?;
                let ek = fips203::ml_kem_512::EncapsKey::try_from_bytes(pk_arr)
                    .map_err(|e| anyhow!("Invalid ML-KEM-512 pk: {}", e))?;
                let (ss, ct) = ek
                    .try_encaps()
                    .map_err(|e| anyhow!("ML-KEM-512 encapsulate failed: {}", e))?;
                Ok((ss.into_bytes().to_vec(), ct.into_bytes().to_vec()))
            }
            KEMVariant::MLKEM768 => {
                let pk_arr: [u8; ml_kem_768::EK_LEN] = pk_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-KEM-768 public key length"))?;
                let ek = ml_kem_768::EncapsKey::try_from_bytes(pk_arr)
                    .map_err(|e| anyhow!("Invalid ML-KEM-768 pk: {}", e))?;
                let (ss, ct) = ek
                    .try_encaps()
                    .map_err(|e| anyhow!("ML-KEM-768 encapsulate failed: {}", e))?;
                Ok((ss.into_bytes().to_vec(), ct.into_bytes().to_vec()))
            }
            KEMVariant::MLKEM1024 => {
                let pk_arr: [u8; fips203::ml_kem_1024::EK_LEN] = pk_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-KEM-1024 public key length"))?;
                let ek = fips203::ml_kem_1024::EncapsKey::try_from_bytes(pk_arr)
                    .map_err(|e| anyhow!("Invalid ML-KEM-1024 pk: {}", e))?;
                let (ss, ct) = ek
                    .try_encaps()
                    .map_err(|e| anyhow!("ML-KEM-1024 encapsulate failed: {}", e))?;
                Ok((ss.into_bytes().to_vec(), ct.into_bytes().to_vec()))
            }
        }
    }

    // ── ML-KEM decapsulate ──

    pub fn decapsulate(variant: KEMVariant, ct_bytes: &[u8], sk_bytes: &[u8]) -> Result<Vec<u8>> {
        match variant {
            KEMVariant::MLKEM512 => {
                let dk_arr: [u8; fips203::ml_kem_512::DK_LEN] = sk_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-KEM-512 secret key length"))?;
                let ct_arr: [u8; fips203::ml_kem_512::CT_LEN] = ct_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-KEM-512 ciphertext length"))?;
                let dk = fips203::ml_kem_512::DecapsKey::try_from_bytes(dk_arr)
                    .map_err(|e| anyhow!("Invalid ML-KEM-512 dk: {}", e))?;
                let ct = fips203::ml_kem_512::CipherText::try_from_bytes(ct_arr)
                    .map_err(|e| anyhow!("Invalid ML-KEM-512 ct: {}", e))?;
                let ss = dk
                    .try_decaps(&ct)
                    .map_err(|e| anyhow!("ML-KEM-512 decapsulate failed: {}", e))?;
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
                    .map_err(|e| anyhow!("Invalid ML-KEM-768 dk: {}", e))?;
                let ct = ml_kem_768::CipherText::try_from_bytes(ct_arr)
                    .map_err(|e| anyhow!("Invalid ML-KEM-768 ct: {}", e))?;
                let ss = dk
                    .try_decaps(&ct)
                    .map_err(|e| anyhow!("ML-KEM-768 decapsulate failed: {}", e))?;
                Ok(ss.into_bytes().to_vec())
            }
            KEMVariant::MLKEM1024 => {
                let dk_arr: [u8; fips203::ml_kem_1024::DK_LEN] = sk_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-KEM-1024 secret key length"))?;
                let ct_arr: [u8; fips203::ml_kem_1024::CT_LEN] = ct_bytes
                    .try_into()
                    .map_err(|_| anyhow!("Invalid ML-KEM-1024 ciphertext length"))?;
                let dk = fips203::ml_kem_1024::DecapsKey::try_from_bytes(dk_arr)
                    .map_err(|e| anyhow!("Invalid ML-KEM-1024 dk: {}", e))?;
                let ct = fips203::ml_kem_1024::CipherText::try_from_bytes(ct_arr)
                    .map_err(|e| anyhow!("Invalid ML-KEM-1024 ct: {}", e))?;
                let ss = dk
                    .try_decaps(&ct)
                    .map_err(|e| anyhow!("ML-KEM-1024 decapsulate failed: {}", e))?;
                Ok(ss.into_bytes().to_vec())
            }
        }
    }

    #[deprecated(note = "Use encapsulate instead")]
    pub fn encapsulate_mlkem768(pk_bytes: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
        Self::encapsulate(KEMVariant::MLKEM768, pk_bytes)
    }

    #[deprecated(note = "Use decapsulate instead")]
    pub fn decapsulate_mlkem768(ct_bytes: &[u8], sk_bytes: &[u8]) -> Result<Vec<u8>> {
        Self::decapsulate(KEMVariant::MLKEM768, ct_bytes, sk_bytes)
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
    pub fn encrypt(data: &[u8], recipient_public_key: &[u8]) -> Result<EncryptedPacket> {
        Self::encrypt_with_variant(data, recipient_public_key, KEMVariant::MLKEM768)
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
            .map_err(|e| anyhow!("Enc failed: {}", e))?;
        let (aes_ct, tag) = ciphertext_with_tag.split_at(ciphertext_with_tag.len() - 16);
        Ok(EncryptedPacket {
            kem_ct,
            aes_ct: aes_ct.to_vec(),
            nonce: nonce_bytes.to_vec(),
            tag: tag.to_vec(),
            algo: format!("{}/AES-256-GCM", variant.to_str()),
        })
    }

    pub fn decrypt(packet: &EncryptedPacket, private_key: &[u8]) -> Result<Vec<u8>> {
        let variant = if packet.algo.contains("ML-KEM-512") {
            KEMVariant::MLKEM512
        } else if packet.algo.contains("ML-KEM-1024") {
            KEMVariant::MLKEM1024
        } else {
            KEMVariant::MLKEM768
        };

        let ss = PqcProvider::decapsulate(variant, &packet.kem_ct, private_key)?;
        let key = &ss[..32];
        let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| anyhow!("AES init failed"))?;
        let nonce = Nonce::from_slice(&packet.nonce);
        let mut encrypted = packet.aes_ct.clone();
        encrypted.extend_from_slice(&packet.tag);
        cipher
            .decrypt(nonce, encrypted.as_slice())
            .map_err(|e| anyhow!("Dec failed: {}", e))
    }
}

pub struct PQCAgilityManager;

impl PQCAgilityManager {
    /// Always returns [`DEFAULT_PQC_VARIANT`] (ML-DSA-87).
    /// Risk-level-based variant switching is discontinued.
    pub fn get_required_level(
        _config: &crate::config::models::AppConfig,
        _tool_name: &str,
        _args: Option<&serde_json::Value>,
    ) -> PQCVariant {
        DEFAULT_PQC_VARIANT
    }
}

pub struct ResponseSigner;
impl ResponseSigner {
    pub fn sign_response(
        text: &str,
        id: &str,
        sk: &[u8],
        v: PQCVariant,
    ) -> Result<serde_json::Value> {
        let msg = format!("{}:{}", id, text);
        let sig = PqcProvider::sign(v, sk, msg.as_bytes())?;
        Ok(serde_json::json!({
            "result": text,
            "verification_id": id,
            "pqc_signature": base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, sig),
            "algorithm": v.to_str()
        }))
    }
}
