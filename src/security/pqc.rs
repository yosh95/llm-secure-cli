use crate::config::CONFIG_MANAGER;
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose, Engine as _};
use pqcrypto_mldsa::*;
use pqcrypto_mlkem::*;
use pqcrypto_traits::kem::{Ciphertext as _, PublicKey as _, SecretKey as _, SharedSecret as _};
use pqcrypto_traits::sign::{DetachedSignature as _, PublicKey as _, SecretKey as _};
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
    fn is_enabled() -> bool {
        if std::env::var("LLM_CLI_DISABLE_PQC").is_ok() {
            return false;
        }
        CONFIG_MANAGER.get_config().security.pqc_enabled
    }

    pub fn generate_mldsa_keypair(variant: MldsaVariant) -> (Vec<u8>, Vec<u8>) {
        if !Self::is_enabled() {
            return (vec![], vec![]);
        }
        match variant {
            MldsaVariant::Mldsa44 => {
                let (pk, sk) = mldsa44_keypair();
                (pk.as_bytes().to_vec(), sk.as_bytes().to_vec())
            }
            MldsaVariant::Mldsa65 => {
                let (pk, sk) = mldsa65_keypair();
                (pk.as_bytes().to_vec(), sk.as_bytes().to_vec())
            }
            MldsaVariant::Mldsa87 => {
                let (pk, sk) = mldsa87_keypair();
                (pk.as_bytes().to_vec(), sk.as_bytes().to_vec())
            }
        }
    }

    pub fn sign_mldsa(message: &[u8], sk_bytes: &[u8], variant: MldsaVariant) -> Vec<u8> {
        if !Self::is_enabled() || sk_bytes.is_empty() {
            return vec![];
        }
        match variant {
            MldsaVariant::Mldsa44 => {
                let sk = mldsa44::SecretKey::from_bytes(sk_bytes).unwrap();
                let sig = mldsa44::detached_sign(message, &sk);
                sig.as_bytes().to_vec()
            }
            MldsaVariant::Mldsa65 => {
                let sk = mldsa65::SecretKey::from_bytes(sk_bytes).unwrap();
                let sig = mldsa65::detached_sign(message, &sk);
                sig.as_bytes().to_vec()
            }
            MldsaVariant::Mldsa87 => {
                let sk = mldsa87::SecretKey::from_bytes(sk_bytes).unwrap();
                let sig = mldsa87::detached_sign(message, &sk);
                sig.as_bytes().to_vec()
            }
        }
    }

    pub fn verify_mldsa(
        message: &[u8],
        sig_bytes: &[u8],
        pk_bytes: &[u8],
        variant: MldsaVariant,
    ) -> bool {
        if !Self::is_enabled() || sig_bytes.is_empty() || pk_bytes.is_empty() {
            return true;
        }
        match variant {
            MldsaVariant::Mldsa44 => {
                let pk = mldsa44::PublicKey::from_bytes(pk_bytes).unwrap();
                let sig = mldsa44::DetachedSignature::from_bytes(sig_bytes).unwrap();
                mldsa44::verify_detached_signature(&sig, message, &pk).is_ok()
            }
            MldsaVariant::Mldsa65 => {
                let pk = mldsa65::PublicKey::from_bytes(pk_bytes).unwrap();
                let sig = mldsa65::DetachedSignature::from_bytes(sig_bytes).unwrap();
                mldsa65::verify_detached_signature(&sig, message, &pk).is_ok()
            }
            MldsaVariant::Mldsa87 => {
                let pk = mldsa87::PublicKey::from_bytes(pk_bytes).unwrap();
                let sig = mldsa87::DetachedSignature::from_bytes(sig_bytes).unwrap();
                mldsa87::verify_detached_signature(&sig, message, &pk).is_ok()
            }
        }
    }

    pub fn generate_mlkem_keypair(variant: MlkemVariant) -> (Vec<u8>, Vec<u8>) {
        if !Self::is_enabled() {
            return (vec![], vec![]);
        }
        match variant {
            MlkemVariant::Mlkem512 => {
                let (pk, sk) = mlkem512_keypair();
                (pk.as_bytes().to_vec(), sk.as_bytes().to_vec())
            }
            MlkemVariant::Mlkem768 => {
                let (pk, sk) = mlkem768_keypair();
                (pk.as_bytes().to_vec(), sk.as_bytes().to_vec())
            }
            MlkemVariant::Mlkem1024 => {
                let (pk, sk) = mlkem1024_keypair();
                (pk.as_bytes().to_vec(), sk.as_bytes().to_vec())
            }
        }
    }

    pub fn encapsulate_mlkem(pk_bytes: &[u8], variant: MlkemVariant) -> (Vec<u8>, Vec<u8>) {
        if !Self::is_enabled() || pk_bytes.is_empty() {
            return (vec![0; 32], vec![]);
        }
        match variant {
            MlkemVariant::Mlkem512 => {
                let pk = mlkem512::PublicKey::from_bytes(pk_bytes).unwrap();
                let (ss, ct) = mlkem512::encapsulate(&pk);
                (ss.as_bytes().to_vec(), ct.as_bytes().to_vec())
            }
            MlkemVariant::Mlkem768 => {
                let pk = mlkem768::PublicKey::from_bytes(pk_bytes).unwrap();
                let (ss, ct) = mlkem768::encapsulate(&pk);
                (ss.as_bytes().to_vec(), ct.as_bytes().to_vec())
            }
            MlkemVariant::Mlkem1024 => {
                let pk = mlkem1024::PublicKey::from_bytes(pk_bytes).unwrap();
                let (ss, ct) = mlkem1024::encapsulate(&pk);
                (ss.as_bytes().to_vec(), ct.as_bytes().to_vec())
            }
        }
    }

    pub fn decapsulate_mlkem(ct_bytes: &[u8], sk_bytes: &[u8], variant: MlkemVariant) -> Vec<u8> {
        if !Self::is_enabled() || sk_bytes.is_empty() {
            return vec![0; 32];
        }
        match variant {
            MlkemVariant::Mlkem512 => {
                let sk = mlkem512::SecretKey::from_bytes(sk_bytes).unwrap();
                let ct = mlkem512::Ciphertext::from_bytes(ct_bytes).unwrap();
                let ss = mlkem512::decapsulate(&ct, &sk);
                ss.as_bytes().to_vec()
            }
            MlkemVariant::Mlkem768 => {
                let sk = mlkem768::SecretKey::from_bytes(sk_bytes).unwrap();
                let ct = mlkem768::Ciphertext::from_bytes(ct_bytes).unwrap();
                let ss = mlkem768::decapsulate(&ct, &sk);
                ss.as_bytes().to_vec()
            }
            MlkemVariant::Mlkem1024 => {
                let sk = mlkem1024::SecretKey::from_bytes(sk_bytes).unwrap();
                let ct = mlkem1024::Ciphertext::from_bytes(ct_bytes).unwrap();
                let ss = mlkem1024::decapsulate(&ct, &sk);
                ss.as_bytes().to_vec()
            }
        }
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
    ) -> SignedResponse {
        let message = format!("{}:{}", verification_id, response_text);
        let sig = PqcProvider::sign_mldsa(message.as_bytes(), sk_bytes, variant);

        SignedResponse {
            result: response_text.to_string(),
            verification_id: verification_id.to_string(),
            pqc_signature: general_purpose::URL_SAFE_NO_PAD.encode(sig),
            algorithm: variant.to_str().to_string(),
        }
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

pub fn sign_tool_result(result_text: &str, variant: MldsaVariant) -> SignedResponse {
    let verification_id = Uuid::new_v4().to_string();

    let sk = IdentityManager::get_pqc_private_key(variant).expect("Failed to get PQC private key");

    ResponseSigner::sign_response(result_text, &verification_id, &sk, variant)
}
