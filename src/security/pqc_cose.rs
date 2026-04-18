use crate::security::pqc::{MldsaVariant, PqcProvider};
use ciborium::Value;
use rsa::pkcs1v15::Pkcs1v15Sign;
use rsa::{
    pkcs8::{DecodePrivateKey, DecodePublicKey},
    RsaPrivateKey, RsaPublicKey,
};
use serde_json::Value as JsonValue;
use sha2::Digest;
use std::collections::HashMap;

// COSE constants
const COSE_ALG_RS256: i64 = -257;
const COSE_ALG_MLDSA: i64 = -48;
const COSE_HEADER_ALG: i64 = 1;
const COSE_SIGN_TAG: u64 = 98;

pub struct HybridSigner;

impl HybridSigner {
    pub fn create_hybrid_token(
        payload: &JsonValue,
        rsa_private_key_pem: &str,
        pqc_private_key: &[u8],
        variant: MldsaVariant,
    ) -> Vec<u8> {
        let payload_bytes = serde_json::to_vec(payload).unwrap();
        let body_protected = Self::encode_header_map(HashMap::new());

        // --- Signer 0: RSA ---
        let mut rsa_header = HashMap::new();
        rsa_header.insert(COSE_HEADER_ALG, Value::Integer(COSE_ALG_RS256.into()));
        let rsa_sign_protected = Self::encode_header_map(rsa_header);

        let rsa_tbs =
            Self::build_sig_structure(&body_protected, &rsa_sign_protected, &payload_bytes);
        let rsa_priv = RsaPrivateKey::from_pkcs8_pem(rsa_private_key_pem)
            .expect("Failed to load RSA private key");

        let digest = sha2::Sha256::digest(&rsa_tbs);
        let rsa_sig = rsa_priv
            .sign(Pkcs1v15Sign::new::<sha2::Sha256>(), &digest)
            .expect("RSA signing failed");

        let rsa_signature_entry = Value::Array(vec![
            Value::Bytes(rsa_sign_protected),
            Value::Map(vec![]),
            Value::Bytes(rsa_sig),
        ]);

        // --- Signer 1: ML-DSA ---
        let mut pqc_header = HashMap::new();
        pqc_header.insert(COSE_HEADER_ALG, Value::Integer(COSE_ALG_MLDSA.into()));
        let pqc_sign_protected = Self::encode_header_map(pqc_header);

        let pqc_uhdr = vec![(
            Value::Integer(4.into()),
            Value::Bytes(variant.to_str().as_bytes().to_vec()),
        )];

        let pqc_tbs =
            Self::build_sig_structure(&body_protected, &pqc_sign_protected, &payload_bytes);
        let pqc_sig = PqcProvider::sign_mldsa(&pqc_tbs, pqc_private_key, variant);

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
            Value::Array(vec![rsa_signature_entry, pqc_signature_entry]),
        ]);

        let mut encoded = Vec::new();
        ciborium::ser::into_writer(
            &Value::Tag(COSE_SIGN_TAG, Box::new(cose_sign_value)),
            &mut encoded,
        )
        .unwrap();
        encoded
    }

    pub fn verify_hybrid_token(
        cose_token: &[u8],
        rsa_public_key_pem: &str,
        pqc_public_key_provider: impl Fn(MldsaVariant) -> Vec<u8>,
    ) -> Option<JsonValue> {
        let value: Value = ciborium::de::from_reader(cose_token).ok()?;
        let (tag, structure) = match value {
            Value::Tag(tag, box_val) => (tag, *box_val),
            _ => return None,
        };

        if tag != COSE_SIGN_TAG {
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
            return None;
        }

        // Signer 0: RSA
        let rsa_entry = match &signatures[0] {
            Value::Array(arr) if arr.len() == 3 => arr,
            _ => return None,
        };
        let rsa_sign_protected = match &rsa_entry[0] {
            Value::Bytes(b) => b,
            _ => return None,
        };
        let rsa_sig = match &rsa_entry[2] {
            Value::Bytes(b) => b,
            _ => return None,
        };

        let rsa_phdr: Value = ciborium::de::from_reader(rsa_sign_protected.as_slice()).ok()?;
        let rsa_alg = match rsa_phdr {
            Value::Map(m) => m
                .into_iter()
                .find(|(k, _)| k == &Value::Integer(COSE_HEADER_ALG.into()))
                .map(|(_, v)| v),
            _ => None,
        };
        if rsa_alg != Some(Value::Integer(COSE_ALG_RS256.into())) {
            return None;
        }

        let rsa_tbs = Self::build_sig_structure(body_protected, rsa_sign_protected, payload_bytes);
        let rsa_pub = RsaPublicKey::from_public_key_pem(rsa_public_key_pem).ok()?;
        let digest = sha2::Sha256::digest(&rsa_tbs);
        rsa_pub
            .verify(Pkcs1v15Sign::new::<sha2::Sha256>(), &digest, rsa_sig)
            .ok()?;

        // Signer 1: ML-DSA
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
            return None;
        }

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
