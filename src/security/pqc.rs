use base64::{engine::general_purpose, Engine as _};
use pqcrypto_mldsa::*;
use pqcrypto_mlkem::*;
use pqcrypto_traits::kem::{Ciphertext as _, PublicKey as _, SecretKey as _, SharedSecret as _};
use pqcrypto_traits::sign::{DetachedSignature as _, PublicKey as _, SecretKey as _};
use serde::{Deserialize, Serialize};

pub enum MldsaVariant {
    Mldsa44,
    Mldsa65,
    Mldsa87,
}

pub struct PqcProvider;

impl PqcProvider {
    pub fn generate_mldsa_65_keypair() -> (Vec<u8>, Vec<u8>) {
        let (pk, sk) = mldsa65_keypair();
        (pk.as_bytes().to_vec(), sk.as_bytes().to_vec())
    }

    pub fn sign_mldsa_65(message: &[u8], sk_bytes: &[u8]) -> Vec<u8> {
        let sk = mldsa65::SecretKey::from_bytes(sk_bytes).unwrap();
        let sig = mldsa65::detached_sign(message, &sk);
        sig.as_bytes().to_vec()
    }

    pub fn verify_mldsa_65(message: &[u8], sig_bytes: &[u8], pk_bytes: &[u8]) -> bool {
        let pk = mldsa65::PublicKey::from_bytes(pk_bytes).unwrap();
        let sig = mldsa65::DetachedSignature::from_bytes(sig_bytes).unwrap();
        mldsa65::verify_detached_signature(&sig, message, &pk).is_ok()
    }

    pub fn generate_mlkem_768_keypair() -> (Vec<u8>, Vec<u8>) {
        let (pk, sk) = mlkem768_keypair();
        (pk.as_bytes().to_vec(), sk.as_bytes().to_vec())
    }

    pub fn encapsulate_mlkem_768(pk_bytes: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let pk = mlkem768::PublicKey::from_bytes(pk_bytes).unwrap();
        let (ss, ct) = mlkem768::encapsulate(&pk);
        (ss.as_bytes().to_vec(), ct.as_bytes().to_vec())
    }

    pub fn decapsulate_mlkem_768(ct_bytes: &[u8], sk_bytes: &[u8]) -> Vec<u8> {
        let sk = mlkem768::SecretKey::from_bytes(sk_bytes).unwrap();
        let ct = mlkem768::Ciphertext::from_bytes(ct_bytes).unwrap();
        let ss = mlkem768::decapsulate(&ct, &sk);
        ss.as_bytes().to_vec()
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
    ) -> SignedResponse {
        let message = format!("{}:{}", verification_id, response_text);
        let sig = PqcProvider::sign_mldsa_65(message.as_bytes(), sk_bytes);

        SignedResponse {
            result: response_text.to_string(),
            verification_id: verification_id.to_string(),
            pqc_signature: general_purpose::URL_SAFE_NO_PAD.encode(sig),
            algorithm: "ML-DSA-65".to_string(),
        }
    }

    pub fn verify_response(signed: &SignedResponse, pk_bytes: &[u8]) -> bool {
        let message = format!("{}:{}", signed.verification_id, signed.result);
        let sig = general_purpose::URL_SAFE_NO_PAD
            .decode(&signed.pqc_signature)
            .unwrap_or_default();
        PqcProvider::verify_mldsa_65(message.as_bytes(), &sig, pk_bytes)
    }
}
