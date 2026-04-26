use crate::config::CONFIG_MANAGER;
use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use base64::{Engine as _, engine::general_purpose};
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

impl MlkemVariant {
    pub fn to_str(&self) -> &'static str {
        match self {
            MlkemVariant::Mlkem512 => "ML-KEM-512",
            MlkemVariant::Mlkem768 => "ML-KEM-768",
            MlkemVariant::Mlkem1024 => "ML-KEM-1024",
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
        log::debug!("PQC: Generating ML-DSA keypair (variant: {:?})", variant);
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
        log::debug!(
            "PQC: Signing message with ML-DSA (variant: {:?}, msg_len: {})",
            variant,
            message.len()
        );
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
            log::debug!("PQC: ML-DSA verification failed (empty signature or public key)");
            return false;
        }
        log::debug!(
            "PQC: Verifying ML-DSA signature (variant: {:?}, msg_len: {})",
            variant,
            message.len()
        );
        Self::ensure_init();
        let saorsa_variant = Self::map_mldsa_variant(variant);
        let ops = MlDsa::new(saorsa_variant);
        let pk = match MlDsaPublicKey::from_bytes(saorsa_variant, pk_bytes) {
            Ok(pk) => pk,
            Err(_) => {
                log::debug!("PQC: ML-DSA verification failed (invalid public key)");
                return false;
            }
        };
        let sig = match MlDsaSignature::from_bytes(saorsa_variant, sig_bytes) {
            Ok(sig) => sig,
            Err(_) => {
                log::debug!("PQC: ML-DSA verification failed (invalid signature format)");
                return false;
            }
        };
        let result = ops.verify(&pk, message, &sig).unwrap_or_default();
        log::debug!("PQC: ML-DSA verification result: {}", result);
        result
    }

    pub fn generate_mlkem_keypair(variant: MlkemVariant) -> (Vec<u8>, Vec<u8>) {
        log::debug!("PQC: Generating ML-KEM keypair (variant: {:?})", variant);
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
        log::debug!("PQC: ML-KEM encapsulation (variant: {:?})", variant);
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
        log::debug!("PQC: ML-KEM decapsulation (variant: {:?})", variant);
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
    pub fn encrypt(data: &[u8], recipient_public_key: &[u8]) -> anyhow::Result<EncryptedPacket> {
        log::debug!("SecureStorage: Encrypting {} bytes of data", data.len());
        let (shared_secret, kem_ct) =
            PqcProvider::encapsulate_mlkem(recipient_public_key, MlkemVariant::Mlkem768);

        if shared_secret == vec![0; 32] && recipient_public_key.is_empty() {
            return Err(anyhow::anyhow!(
                "Encryption failed: Recipient public key is empty"
            ));
        }

        // Use first 32 bytes of shared secret for AES-256
        let key = &shared_secret[..32];
        let cipher = Aes256Gcm::new_from_slice(key)
            .map_err(|_| anyhow::anyhow!("Failed to initialize AES-GCM"))?;

        let nonce_bytes: [u8; 12] = rand::random();
        let nonce = Nonce::from(nonce_bytes);

        let ciphertext_with_tag = cipher
            .encrypt(&nonce, data)
            .map_err(|e| anyhow::anyhow!("AES encryption failure: {}", e))?;

        if ciphertext_with_tag.len() < 16 {
            return Err(anyhow::anyhow!("Encryption failed: Ciphertext too short"));
        }

        let tag_start = ciphertext_with_tag.len() - 16;
        let aes_ct = &ciphertext_with_tag[..tag_start];
        let tag = &ciphertext_with_tag[tag_start..];

        log::debug!(
            "SecureStorage: Encryption complete (KEM CT len: {}, AES CT len: {})",
            kem_ct.len(),
            aes_ct.len()
        );

        Ok(EncryptedPacket {
            kem_ct: general_purpose::STANDARD.encode(kem_ct),
            aes_ct: general_purpose::STANDARD.encode(aes_ct),
            nonce: general_purpose::STANDARD.encode(nonce_bytes),
            tag: general_purpose::STANDARD.encode(tag),
            algo: "ML-KEM-768/AES-256-GCM".to_string(),
        })
    }

    pub fn decrypt(packet: &EncryptedPacket, private_key: &[u8]) -> anyhow::Result<Vec<u8>> {
        log::debug!("SecureStorage: Decrypting packet (algo: {})", packet.algo);

        let kem_ct = general_purpose::STANDARD
            .decode(&packet.kem_ct)
            .map_err(|e| anyhow::anyhow!("Invalid KEM ciphertext encoding: {}", e))?;

        let shared_secret =
            PqcProvider::decapsulate_mlkem(&kem_ct, private_key, MlkemVariant::Mlkem768);

        if shared_secret == vec![0; 32] && private_key.is_empty() {
            return Err(anyhow::anyhow!("Decryption failed: Private key is empty"));
        }

        let key = &shared_secret[..32];
        let cipher = Aes256Gcm::new_from_slice(key)
            .map_err(|_| anyhow::anyhow!("Failed to initialize AES-GCM"))?;

        let nonce_bytes = general_purpose::STANDARD
            .decode(&packet.nonce)
            .map_err(|e| anyhow::anyhow!("Invalid nonce encoding: {}", e))?;

        let nonce = Nonce::from(
            <[u8; 12]>::try_from(nonce_bytes)
                .map_err(|_| anyhow::anyhow!("Invalid nonce length"))?,
        );

        let aes_ct = general_purpose::STANDARD
            .decode(&packet.aes_ct)
            .map_err(|e| anyhow::anyhow!("Invalid AES ciphertext encoding: {}", e))?;

        let tag = general_purpose::STANDARD
            .decode(&packet.tag)
            .map_err(|e| anyhow::anyhow!("Invalid tag encoding: {}", e))?;

        let mut combined = aes_ct;
        combined.extend_from_slice(&tag);

        let decrypted = cipher
            .decrypt(&nonce, combined.as_slice())
            .map_err(|e| anyhow::anyhow!("AES decryption failure: {}", e))?;

        log::debug!(
            "SecureStorage: Decryption successful ({} bytes)",
            decrypted.len()
        );
        Ok(decrypted)
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
