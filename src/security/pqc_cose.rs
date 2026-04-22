use crate::security::pqc::{MldsaVariant, PqcProvider};
use ciborium::Value;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use pkcs8::{DecodePrivateKey, DecodePublicKey};
use serde_json::Value as JsonValue;
use std::collections::HashMap;

// COSE constants
const COSE_ALG_EDDSA: i64 = -8;
const COSE_ALG_MLDSA: i64 = -48;
const COSE_HEADER_ALG: i64 = 1;
const COSE_SIGN_TAG: u64 = 98;

pub struct HybridSigner;

impl HybridSigner {
    pub fn create_hybrid_token(
        payload: &JsonValue,
        classical_private_key_pem: &str,
        pqc_private_key: &[u8],
        variant: MldsaVariant,
    ) -> Vec<u8> {
        log::debug!(
            "HybridSigner: Creating hybrid COSE token (PQC variant: {:?})",
            variant
        );
        let payload_bytes = serde_json::to_vec(payload).unwrap();
        let body_protected = Self::encode_header_map(HashMap::new());

        // --- Signer 0: Ed25519 (Classical) ---
        log::debug!("HybridSigner: Generating EdDSA signature");
        let mut ed_header = HashMap::new();
        ed_header.insert(COSE_HEADER_ALG, Value::Integer(COSE_ALG_EDDSA.into()));
        let ed_sign_protected = Self::encode_header_map(ed_header);

        let ed_tbs = Self::build_sig_structure(&body_protected, &ed_sign_protected, &payload_bytes);
        let signing_key = SigningKey::from_pkcs8_pem(classical_private_key_pem)
            .expect("Failed to load Ed25519 private key");

        let ed_sig = signing_key.sign(&ed_tbs);

        let classical_signature_entry = Value::Array(vec![
            Value::Bytes(ed_sign_protected),
            Value::Map(vec![]),
            Value::Bytes(ed_sig.to_vec()),
        ]);

        // --- Signer 1: ML-DSA ---
        log::debug!("HybridSigner: Generating ML-DSA signature");
        let mut pqc_header = HashMap::new();
        pqc_header.insert(COSE_HEADER_ALG, Value::Integer(COSE_ALG_MLDSA.into()));
        let pqc_sign_protected = Self::encode_header_map(pqc_header);

        let pqc_uhdr = vec![(
            Value::Integer(4.into()),
            Value::Bytes(variant.to_str().as_bytes().to_vec()),
        )];

        let pqc_tbs =
            Self::build_sig_structure(&body_protected, &pqc_sign_protected, &payload_bytes);
        let pqc_sig =
            PqcProvider::sign_mldsa(&pqc_tbs, pqc_private_key, variant).unwrap_or_default();

        let pqc_signature_entry = Value::Array(vec![
            Value::Bytes(pqc_sign_protected),
            Value::Map(pqc_uhdr),
            Value::Bytes(pqc_sig),
        ]);

        // --- Assemble COSE_Sign ---
        let cose_sign_value = Value::Array(vec![
            Value::Bytes(body_protected),
            Value::Map(vec![]),
            Value::Bytes(payload_bytes),
            Value::Array(vec![classical_signature_entry, pqc_signature_entry]),
        ]);

        let mut encoded = Vec::new();
        ciborium::ser::into_writer(
            &Value::Tag(COSE_SIGN_TAG, Box::new(cose_sign_value)),
            &mut encoded,
        )
        .unwrap();
        log::debug!(
            "HybridSigner: Hybrid token created ({} bytes)",
            encoded.len()
        );
        encoded
    }

    pub fn verify_hybrid_token(
        cose_token: &[u8],
        classical_public_key_pem: &str,
        pqc_public_key_provider: impl Fn(MldsaVariant) -> Vec<u8>,
    ) -> Option<JsonValue> {
        log::debug!(
            "HybridSigner: Verifying hybrid COSE token ({} bytes)",
            cose_token.len()
        );
        let value: Value = ciborium::de::from_reader(cose_token).ok()?;
        let (tag, structure) = match value {
            Value::Tag(tag, box_val) => (tag, *box_val),
            _ => {
                log::debug!("HybridSigner: Verification failed (invalid COSE tag)");
                return None;
            }
        };

        if tag != COSE_SIGN_TAG {
            log::debug!(
                "HybridSigner: Verification failed (not a COSE_Sign tag: {})",
                tag
            );
            return None;
        }

        let elements = match structure {
            Value::Array(arr) => arr,
            _ => return None,
        };

        if elements.len() != 4 {
            return None;
        }

        let body_protected = match &elements[0] {
            Value::Bytes(b) => b,
            _ => return None,
        };
        let payload_bytes = match &elements[2] {
            Value::Bytes(b) => b,
            _ => return None,
        };
        let signatures = match &elements[3] {
            Value::Array(arr) => arr,
            _ => return None,
        };

        if signatures.len() < 2 {
            log::debug!("HybridSigner: Verification failed (less than 2 signatures found)");
            return None;
        }

        // Signer 0: Ed25519 (Classical)
        log::debug!("HybridSigner: Verifying EdDSA signature");
        let classical_entry = match &signatures[0] {
            Value::Array(arr) if arr.len() == 3 => arr,
            _ => return None,
        };
        let classical_sign_protected = match &classical_entry[0] {
            Value::Bytes(b) => b,
            _ => return None,
        };
        let classical_sig_bytes = match &classical_entry[2] {
            Value::Bytes(b) => b,
            _ => return None,
        };

        let classical_phdr: Value =
            ciborium::de::from_reader(classical_sign_protected.as_slice()).ok()?;
        let classical_alg = match classical_phdr {
            Value::Map(m) => m
                .into_iter()
                .find(|(k, _)| k == &Value::Integer(COSE_HEADER_ALG.into()))
                .map(|(_, v)| v),
            _ => None,
        };
        if classical_alg != Some(Value::Integer(COSE_ALG_EDDSA.into())) {
            log::debug!("HybridSigner: Classical verification failed (unsupported algorithm)");
            return None;
        }

        let classical_tbs =
            Self::build_sig_structure(body_protected, classical_sign_protected, payload_bytes);
        let verifying_key = VerifyingKey::from_public_key_pem(classical_public_key_pem).ok()?;
        let classical_sig = Signature::from_slice(classical_sig_bytes).ok()?;

        if let Err(e) = verifying_key.verify(&classical_tbs, &classical_sig) {
            log::debug!("HybridSigner: Classical verification failed: {:?}", e);
            return None;
        }
        log::debug!("HybridSigner: Classical verification successful");

        // Signer 1: ML-DSA
        log::debug!("HybridSigner: Verifying ML-DSA signature");
        let pqc_entry = match &signatures[1] {
            Value::Array(arr) if arr.len() == 3 => arr,
            _ => return None,
        };
        let pqc_sign_protected = match &pqc_entry[0] {
            Value::Bytes(b) => b,
            _ => return None,
        };
        let pqc_uhdr = match &pqc_entry[1] {
            Value::Map(m) => m,
            _ => return None,
        };
        let pqc_sig = match &pqc_entry[2] {
            Value::Bytes(b) => b,
            _ => return None,
        };

        let pqc_phdr: Value = ciborium::de::from_reader(pqc_sign_protected.as_slice()).ok()?;
        let pqc_alg = match pqc_phdr {
            Value::Map(m) => m
                .into_iter()
                .find(|(k, _)| k == &Value::Integer(COSE_HEADER_ALG.into()))
                .map(|(_, v)| v),
            _ => None,
        };
        if pqc_alg != Some(Value::Integer(COSE_ALG_MLDSA.into())) {
            log::debug!("HybridSigner: ML-DSA verification failed (unsupported algorithm)");
            return None;
        }

        let variant_str = pqc_uhdr
            .iter()
            .find(|(k, _)| k == &Value::Integer(4.into()))
            .and_then(|(_, v)| match v {
                Value::Bytes(b) => String::from_utf8(b.clone()).ok(),
                Value::Text(t) => Some(t.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "ML-DSA-65".to_string());

        use std::str::FromStr;
        let variant = MldsaVariant::from_str(&variant_str).unwrap_or(MldsaVariant::Mldsa65);
        let pqc_pub = pqc_public_key_provider(variant);

        let pqc_tbs = Self::build_sig_structure(body_protected, pqc_sign_protected, payload_bytes);
        if !PqcProvider::verify_mldsa(&pqc_tbs, pqc_sig, &pqc_pub, variant) {
            log::debug!("HybridSigner: ML-DSA verification failed (invalid signature)");
            return None;
        }
        log::debug!(
            "HybridSigner: ML-DSA verification successful (variant: {:?})",
            variant
        );

        log::debug!("HybridSigner: Full hybrid token verification successful");
        serde_json::from_slice(payload_bytes).ok()
    }

    fn encode_header_map(map: HashMap<i64, Value>) -> Vec<u8> {
        let mut v = Vec::new();
        let map_val = Value::Map(
            map.into_iter()
                .map(|(k, v)| (Value::Integer(k.into()), v))
                .collect(),
        );
        ciborium::ser::into_writer(&map_val, &mut v).unwrap();
        v
    }

    fn build_sig_structure(
        body_protected: &[u8],
        sign_protected: &[u8],
        payload: &[u8],
    ) -> Vec<u8> {
        let structure = Value::Array(vec![
            Value::Text("Signature".to_string()),
            Value::Bytes(body_protected.to_vec()),
            Value::Bytes(sign_protected.to_vec()),
            Value::Bytes(vec![]), // external_aad
            Value::Bytes(payload.to_vec()),
        ]);
        let mut v = Vec::new();
        ciborium::ser::into_writer(&structure, &mut v).unwrap();
        v
    }
}
