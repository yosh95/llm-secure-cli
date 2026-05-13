use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use anyhow::{Result, anyhow};
use rand::RngCore;
use saorsa_pqc::api::{
    MlDsa, MlDsaPublicKey, MlDsaSecretKey, MlDsaSignature, MlDsaVariant as SaorsaMldsaVariant,
    MlKem, MlKemCiphertext, MlKemPublicKey, MlKemSecretKey, MlKemVariant as SaorsaMlkemVariant,
};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

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

pub struct PqcProvider;

impl PqcProvider {
    fn map_mldsa_variant(v: PQCVariant) -> SaorsaMldsaVariant {
        match v {
            PQCVariant::MLDSA44 => SaorsaMldsaVariant::MlDsa44,
            PQCVariant::MLDSA65 => SaorsaMldsaVariant::MlDsa65,
            PQCVariant::MLDSA87 => SaorsaMldsaVariant::MlDsa87,
        }
    }

    fn map_mlkem_variant(v: KEMVariant) -> SaorsaMlkemVariant {
        match v {
            KEMVariant::MLKEM512 => SaorsaMlkemVariant::MlKem512,
            KEMVariant::MLKEM768 => SaorsaMlkemVariant::MlKem768,
            KEMVariant::MLKEM1024 => SaorsaMlkemVariant::MlKem1024,
        }
    }

    pub fn generate_keypair(variant: PQCVariant) -> Result<(Vec<u8>, Vec<u8>)> {
        let v = Self::map_mldsa_variant(variant);
        let ops = MlDsa::new(v);
        let (pk, sk) = ops
            .generate_keypair()
            .map_err(|_| anyhow!("PQC keygen failed"))?;
        Ok((pk.to_bytes(), sk.to_bytes()))
    }

    pub fn generate_kem_keypair(variant: KEMVariant) -> Result<(Vec<u8>, Vec<u8>)> {
        let v = Self::map_mlkem_variant(variant);
        let ops = MlKem::new(v);
        let (pk, sk) = ops
            .generate_keypair()
            .map_err(|_| anyhow!("PQC KEM keygen failed"))?;
        Ok((pk.to_bytes(), sk.to_bytes()))
    }

    pub fn sign(variant: PQCVariant, sk_bytes: &[u8], message: &[u8]) -> Result<Vec<u8>> {
        let v = Self::map_mldsa_variant(variant);
        let ops = MlDsa::new(v);
        let sk = MlDsaSecretKey::from_bytes(v, sk_bytes).map_err(|_| anyhow!("Invalid PQC sk"))?;
        let sig = ops
            .sign(&sk, message)
            .map_err(|_| anyhow!("PQC sign failed"))?;
        Ok(sig.to_bytes())
    }

    pub fn verify(
        variant: PQCVariant,
        pk_bytes: &[u8],
        message: &[u8],
        sig_bytes: &[u8],
    ) -> Result<()> {
        let v = Self::map_mldsa_variant(variant);
        let ops = MlDsa::new(v);
        let pk = MlDsaPublicKey::from_bytes(v, pk_bytes).map_err(|_| anyhow!("Invalid PQC pk"))?;
        let sig =
            MlDsaSignature::from_bytes(v, sig_bytes).map_err(|_| anyhow!("Invalid PQC sig"))?;
        if ops.verify(&pk, message, &sig).unwrap_or(false) {
            Ok(())
        } else {
            Err(anyhow!("PQC Verification failed"))
        }
    }

    // Existing code fallback
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

    pub fn encapsulate(variant: KEMVariant, pk_bytes: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
        let v = Self::map_mlkem_variant(variant);
        let ops = MlKem::new(v);
        let pk = MlKemPublicKey::from_bytes(v, pk_bytes).map_err(|_| anyhow!("Invalid PQC pk"))?;
        let (ss, ct) = ops
            .encapsulate(&pk)
            .map_err(|_| anyhow!("PQC encapsulate failed"))?;
        Ok((ss.to_bytes().to_vec(), ct.to_bytes().to_vec()))
    }

    pub fn decapsulate(variant: KEMVariant, ct_bytes: &[u8], sk_bytes: &[u8]) -> Result<Vec<u8>> {
        let v = Self::map_mlkem_variant(variant);
        let ops = MlKem::new(v);
        let sk = MlKemSecretKey::from_bytes(v, sk_bytes).map_err(|_| anyhow!("Invalid PQC sk"))?;
        let ct = MlKemCiphertext::from_bytes(v, ct_bytes).map_err(|_| anyhow!("Invalid PQC ct"))?;
        let ss = ops
            .decapsulate(&sk, &ct)
            .map_err(|_| anyhow!("PQC decapsulate failed"))?;
        Ok(ss.to_bytes().to_vec())
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
    pub fn get_required_level(
        config: &crate::config::models::AppConfig,
        tool_name: &str,
        args: Option<&serde_json::Value>,
    ) -> PQCVariant {
        use crate::security::cass::{CASS_ORCHESTRATOR, RiskLevel};
        let risk = CASS_ORCHESTRATOR.evaluate_risk(tool_name, args, &config.security);
        match risk {
            RiskLevel::Critical | RiskLevel::High => PQCVariant::MLDSA87,
            RiskLevel::Medium => PQCVariant::MLDSA65,
            _ => PQCVariant::MLDSA44,
        }
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
