use crate::config::CONFIG_MANAGER;
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose, Engine as _};
use saorsa_pqc::api::{
    MlDsa, MlDsaPublicKey, MlDsaSecretKey, MlDsaSignature, MlDsaVariant as SaorsaMldsaVariant,
    MlKem, MlKemCiphertext, MlKemPublicKey, MlKemSecretKey, MlKemVariant as SaorsaMlkemVariant,
};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MldsaVariant {
    Mldsa44,
    Mldsa65,
    Mldsa87,
}

impl FromStr for MldsaVariant {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "ML-DSA-44" => Ok(MldsaVariant::Mldsa44),
            "ML-DSA-65" => Ok(MldsaVariant::Mldsa65),
            "ML-DSA-87" => Ok(MldsaVariant::Mldsa87),
            _ => Err(()),
        }
    }
}

impl MldsaVariant {
    pub fn to_str(&self) -> &'static str {
        match self {
            MldsaVariant::Mldsa44 => "ML-DSA-44",
            MldsaVariant::Mldsa65 => "ML-DSA-65",
            MldsaVariant::Mldsa87 => "ML-DSA-87",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MlkemVariant {
    Mlkem512,
    Mlkem768,
    Mlkem1024,
}

impl FromStr for MlkemVariant {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "ML-KEM-512" => Ok(MlkemVariant::Mlkem512),
            "ML-KEM-768" => Ok(MlkemVariant::Mlkem768),
            "ML-KEM-1024" => Ok(MlkemVariant::Mlkem1024),
            _ => Err(()),
        }
    }
}

pub struct PqcProvider;

impl PqcProvider {
    fn ensure_init() {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| {
            let _ = saorsa_pqc::api::init();
        });
    }

    fn map_mldsa_variant(variant: MldsaVariant) -> SaorsaMldsaVariant {
        match variant {
            MldsaVariant::Mldsa44 => SaorsaMldsaVariant::MlDsa44,
            MldsaVariant::Mldsa65 => SaorsaMldsaVariant::MlDsa65,
            MldsaVariant::Mldsa87 => SaorsaMldsaVariant::MlDsa87,
        }
    }

    fn map_mlkem_variant(variant: MlkemVariant) -> SaorsaMlkemVariant {
        match variant {
            MlkemVariant::Mlkem512 => SaorsaMlkemVariant::MlKem512,
            MlkemVariant::Mlkem768 => SaorsaMlkemVariant::MlKem768,
            MlkemVariant::Mlkem1024 => SaorsaMlkemVariant::MlKem1024,
        }
    }

    pub fn generate_mldsa_keypair(variant: MldsaVariant) -> (Vec<u8>, Vec<u8>) {
        Self::ensure_init();
        let saorsa_variant = Self::map_mldsa_variant(variant);
        let ops = MlDsa::new(saorsa_variant);
        let (pk, sk) = ops.generate_keypair().expect("PQC keygen failed");
        (pk.to_bytes(), sk.to_bytes())
    }

    pub fn sign_mldsa(
        message: &[u8],
        sk_bytes: &[u8],
        variant: MldsaVariant,
    ) -> anyhow::Result<Vec<u8>> {
        if sk_bytes.is_empty() {
            return Err(anyhow::anyhow!("PQC Secret Key is empty"));
        }
        Self::ensure_init();
        let saorsa_variant = Self::map_mldsa_variant(variant);
        let ops = MlDsa::new(saorsa_variant);
        let sk = MlDsaSecretKey::from_bytes(saorsa_variant, sk_bytes)
            .map_err(|_| anyhow::anyhow!("Invalid PQC secret key"))?;
        let sig = ops
            .sign(&sk, message)
            .map_err(|_| anyhow::anyhow!("PQC sign failed"))?;
        Ok(sig.to_bytes())
    }

    pub fn verify_mldsa(
        message: &[u8],
        sig_bytes: &[u8],
        pk_bytes: &[u8],
        variant: MldsaVariant,
    ) -> bool {
        if sig_bytes.is_empty() || pk_bytes.is_empty() {
            return false;
        }
        Self::ensure_init();
        let saorsa_variant = Self::map_mldsa_variant(variant);
        let ops = MlDsa::new(saorsa_variant);
        let pk = match MlDsaPublicKey::from_bytes(saorsa_variant, pk_bytes) {
            Ok(pk) => pk,
            Err(_) => return false,
        };
        let sig = match MlDsaSignature::from_bytes(saorsa_variant, sig_bytes) {
            Ok(sig) => sig,
            Err(_) => return false,
        };
        ops.verify(&pk, message, &sig).is_ok()
    }

    pub fn generate_mlkem_keypair(variant: MlkemVariant) -> (Vec<u8>, Vec<u8>) {
        Self::ensure_init();
        let saorsa_variant = Self::map_mlkem_variant(variant);
        let ops = MlKem::new(saorsa_variant);
        let (pk, sk) = ops.generate_keypair().expect("PQC keygen failed");
        (pk.to_bytes(), sk.to_bytes())
    }

    pub fn encapsulate_mlkem(pk_bytes: &[u8], variant: MlkemVariant) -> (Vec<u8>, Vec<u8>) {
        if pk_bytes.is_empty() {
            return (vec![0; 32], vec![]);
        }
        Self::ensure_init();
        let saorsa_variant = Self::map_mlkem_variant(variant);
        let ops = MlKem::new(saorsa_variant);
        let pk =
            MlKemPublicKey::from_bytes(saorsa_variant, pk_bytes).expect("Invalid PQC public key");
        let (ss, ct) = ops.encapsulate(&pk).expect("PQC encapsulate failed");
        (ss.to_bytes().to_vec(), ct.to_bytes().to_vec())
    }

    pub fn decapsulate_mlkem(ct_bytes: &[u8], sk_bytes: &[u8], variant: MlkemVariant) -> Vec<u8> {
        if sk_bytes.is_empty() {
            return vec![0; 32];
        }
        Self::ensure_init();
        let saorsa_variant = Self::map_mlkem_variant(variant);
        let ops = MlKem::new(saorsa_variant);
        let sk =
            MlKemSecretKey::from_bytes(saorsa_variant, sk_bytes).expect("Invalid PQC secret key");
        let ct =
            MlKemCiphertext::from_bytes(saorsa_variant, ct_bytes).expect("Invalid PQC ciphertext");
        let ss = ops.decapsulate(&sk, &ct).expect("PQC decapsulate failed");
        ss.to_bytes().to_vec()
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct EncryptedPacket {
    pub kem_ct: String,
    pub aes_ct: String,
    pub nonce: String,
    pub tag: String,
    pub algo: String,
}

pub struct SecureStorage;

impl SecureStorage {
    pub fn encrypt(data: &[u8], recipient_public_key: &[u8]) -> EncryptedPacket {
        let (shared_secret, kem_ct) =
            PqcProvider::encapsulate_mlkem(recipient_public_key, MlkemVariant::Mlkem768);

        // Use first 32 bytes of shared secret for AES-256
        let key = &shared_secret[..32];
        let cipher = Aes256Gcm::new_from_slice(key).unwrap();

        let nonce_bytes: [u8; 12] = rand::random();
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext_with_tag = cipher.encrypt(nonce, data).expect("encryption failure!");

        let tag_start = ciphertext_with_tag.len() - 16;
        let aes_ct = &ciphertext_with_tag[..tag_start];
        let tag = &ciphertext_with_tag[tag_start..];

        EncryptedPacket {
            kem_ct: general_purpose::STANDARD.encode(kem_ct),
            aes_ct: general_purpose::STANDARD.encode(aes_ct),
            nonce: general_purpose::STANDARD.encode(nonce_bytes),
            tag: general_purpose::STANDARD.encode(tag),
            algo: "ML-KEM-768/AES-256-GCM".to_string(),
        }
    }

    pub fn decrypt(packet: &EncryptedPacket, private_key: &[u8]) -> Vec<u8> {
        let kem_ct = general_purpose::STANDARD.decode(&packet.kem_ct).unwrap();
        let shared_secret =
            PqcProvider::decapsulate_mlkem(&kem_ct, private_key, MlkemVariant::Mlkem768);

        let key = &shared_secret[..32];
        let cipher = Aes256Gcm::new_from_slice(key).unwrap();

        let nonce_bytes = general_purpose::STANDARD.decode(&packet.nonce).unwrap();
        let nonce = Nonce::from_slice(&nonce_bytes);

        let aes_ct = general_purpose::STANDARD.decode(&packet.aes_ct).unwrap();
        let tag = general_purpose::STANDARD.decode(&packet.tag).unwrap();

        let mut combined = aes_ct;
        combined.extend_from_slice(&tag);

        cipher
            .decrypt(nonce, combined.as_slice())
            .expect("decryption failure!")
    }
}

pub struct PQCAgilityManager;

impl PQCAgilityManager {
    pub fn get_required_level(
        tool_name: &str,
        args: Option<&serde_json::Value>,
        environment_risk: &str,
    ) -> MldsaVariant {
        let config = CONFIG_MANAGER.get_config();
        let security_config = &config.security;

        let mut is_sensitive_context = false;
        if let Some(args_val) = args {
            let args_str = args_val.to_string().to_lowercase();
            for pattern in &security_config.scaling_patterns {
                if args_str.contains(&pattern.to_lowercase()) {
                    is_sensitive_context = true;
                    break;
                }
            }
            if !is_sensitive_context {
                for pattern in &security_config.blocked_paths {
                    if args_str.contains(&pattern.to_lowercase()) {
                        is_sensitive_context = true;
                        break;
                    }
                }
            }
        }

        if environment_risk == "high"
            || security_config
                .high_risk_tools
                .contains(&tool_name.to_string())
            || is_sensitive_context
        {
            return MldsaVariant::Mldsa87;
        }

        if security_config
            .medium_risk_tools
            .contains(&tool_name.to_string())
        {
            return MldsaVariant::Mldsa65;
        }

        MldsaVariant::Mldsa44
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SignedResponse {
    pub result: String,
    pub verification_id: String,
    pub pqc_signature: String,
    pub algorithm: String,
}

pub struct ResponseSigner;

impl ResponseSigner {
    pub fn sign_response(
        response_text: &str,
        verification_id: &str,
        sk_bytes: &[u8],
        variant: MldsaVariant,
    ) -> anyhow::Result<SignedResponse> {
        let message = format!("{}:{}", verification_id, response_text);
        let sig = PqcProvider::sign_mldsa(message.as_bytes(), sk_bytes, variant)?;

        Ok(SignedResponse {
            result: response_text.to_string(),
            verification_id: verification_id.to_string(),
            pqc_signature: general_purpose::URL_SAFE_NO_PAD.encode(sig),
            algorithm: variant.to_str().to_string(),
        })
    }

    pub fn verify_response(signed: &SignedResponse, pk_bytes: &[u8]) -> bool {
        let variant = MldsaVariant::from_str(&signed.algorithm).unwrap_or(MldsaVariant::Mldsa65);
        let message = format!("{}:{}", signed.verification_id, signed.result);
        let sig = general_purpose::URL_SAFE_NO_PAD
            .decode(&signed.pqc_signature)
            .unwrap_or_default();
        PqcProvider::verify_mldsa(message.as_bytes(), &sig, pk_bytes, variant)
    }
}

use crate::security::identity::IdentityManager;

pub fn sign_tool_result(
    result_text: &str,
    variant: MldsaVariant,
) -> anyhow::Result<SignedResponse> {
    let verification_id = Uuid::new_v4().to_string();

    let sk = IdentityManager::get_pqc_private_key(variant)?;

    ResponseSigner::sign_response(result_text, &verification_id, &sk, variant)
}
