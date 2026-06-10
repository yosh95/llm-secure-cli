use crate::security::pqc::{PQCVariant, PqcProvider};
use anyhow::Result;
use ciborium::Value;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

pub struct HybridSigner;

impl HybridSigner {
    pub fn create_hybrid_token(
        payload: &[u8],
        ed25519_sk: &[u8],
        pqc_sk: &[u8],
        variant: PQCVariant,
    ) -> Result<Vec<u8>> {
        let signing_key = SigningKey::from_bytes(ed25519_sk.try_into()?);

        // 1. Define Headers
        // body_protected: Empty map {}
        let body_protected = Self::encode_map(vec![])?;

        // 2. Signer 0: Ed25519
        // EdDSA = -8, alg label = 1
        let ed_header = vec![(Value::Integer(1.into()), Value::Integer((-8).into()))];
        let ed_sign_protected = Self::encode_map(ed_header)?;

        let ed_tbs = Self::build_sig_structure(&body_protected, &ed_sign_protected, payload)?;
        let ed_sig = signing_key.sign(&ed_tbs).to_bytes().to_vec();

        let classical_signature_entry = Value::Array(vec![
            Value::Bytes(ed_sign_protected),
            Value::Map(vec![]), // unprotected
            Value::Bytes(ed_sig),
        ]);

        // 3. Signer 1: ML-DSA
        let pqc_alg_id = match variant {
            PQCVariant::MLDSA44 => -85,
            PQCVariant::MLDSA65 => -86,
            PQCVariant::MLDSA87 => -87,
        };
        let pqc_header = vec![(Value::Integer(1.into()), Value::Integer(pqc_alg_id.into()))];
        let pqc_sign_protected = Self::encode_map(pqc_header)?;

        let pqc_tbs = Self::build_sig_structure(&body_protected, &pqc_sign_protected, payload)?;
        let pqc_sig = PqcProvider::sign(variant, pqc_sk, &pqc_tbs)?;

        let pqc_signature_entry = Value::Array(vec![
            Value::Bytes(pqc_sign_protected),
            Value::Map(vec![]), // unprotected
            Value::Bytes(pqc_sig),
        ]);

        // 4. Assemble COSE_Sign structure: [protected, unprotected, payload, [sig1, sig2]]
        let cose_sign_array = Value::Array(vec![
            Value::Bytes(body_protected),
            Value::Map(vec![]),
            Value::Bytes(payload.to_vec()),
            Value::Array(vec![classical_signature_entry, pqc_signature_entry]),
        ]);

        // 5. Wrap in Tag 98 and serialize
        let mut encoded = Vec::new();
        ciborium::into_writer(&Value::Tag(98, Box::new(cose_sign_array)), &mut encoded)?;
        Ok(encoded)
    }

    pub fn verify_hybrid_token<F>(
        token: &[u8],
        classical_pub: &[u8],
        pqc_pub_fetcher: F,
    ) -> Option<serde_json::Value>
    where
        F: FnOnce(PQCVariant) -> Vec<u8>,
    {
        let value: Value = ciborium::from_reader(token).ok()?;
        let (body_protected_bytes, payload, signatures) = if let Value::Tag(98, inner) = value {
            if let Value::Array(arr) = *inner {
                if arr.len() < 4 {
                    return None;
                }
                let p = if let Value::Bytes(ref b) = arr[0] {
                    b.clone()
                } else {
                    return None;
                };
                let payload = if let Value::Bytes(ref b) = arr[2] {
                    b.clone()
                } else {
                    return None;
                };
                let sigs = if let Value::Array(ref s) = arr[3] {
                    s.clone()
                } else {
                    return None;
                };
                (p, payload, sigs)
            } else {
                return None;
            }
        } else {
            return None;
        };

        // 1. Verify Classical Ed25519
        if signatures.is_empty() {
            return None;
        }
        let classical_entry = if let Value::Array(ref sig_arr) = signatures[0] {
            sig_arr
        } else {
            return None;
        };
        if classical_entry.len() < 3 {
            return None;
        }
        let ed_protected = if let Value::Bytes(ref b) = classical_entry[0] {
            b
        } else {
            return None;
        };
        let ed_sig_bytes = if let Value::Bytes(ref b) = classical_entry[2] {
            b
        } else {
            return None;
        };

        let ed_tbs =
            Self::build_sig_structure(&body_protected_bytes, ed_protected, &payload).ok()?;
        let vk_bytes: [u8; 32] = classical_pub.try_into().ok()?;
        let vk = VerifyingKey::from_bytes(&vk_bytes).ok()?;
        let sig = Signature::from_slice(ed_sig_bytes).ok()?;
        vk.verify(&ed_tbs, &sig).ok()?;

        // 2. Verify PQC
        if signatures.len() < 2 {
            return None;
        }
        let pqc_entry = if let Value::Array(ref sig_arr) = signatures[1] {
            sig_arr
        } else {
            return None;
        };
        if pqc_entry.len() < 3 {
            return None;
        }
        let pqc_protected_bytes = if let Value::Bytes(ref b) = pqc_entry[0] {
            b
        } else {
            return None;
        };
        let pqc_sig_bytes = if let Value::Bytes(ref b) = pqc_entry[2] {
            b
        } else {
            return None;
        };

        let pqc_protected: Value = ciborium::from_reader(&pqc_protected_bytes[..]).ok()?;
        let pqc_alg_id: i64 = if let Value::Map(m) = pqc_protected {
            m.into_iter()
                .find(|(k, _)| k == &Value::Integer(1.into()))
                .and_then(|(_, v)| {
                    if let Value::Integer(i) = v {
                        i.try_into().ok()
                    } else {
                        None
                    }
                })?
        } else {
            return None;
        };

        let variant = match pqc_alg_id {
            -85 => PQCVariant::MLDSA44,
            -86 => PQCVariant::MLDSA65,
            -87 => PQCVariant::MLDSA87,
            _ => return None,
        };

        let pqc_pub = pqc_pub_fetcher(variant);
        let pqc_tbs =
            Self::build_sig_structure(&body_protected_bytes, pqc_protected_bytes, &payload).ok()?;

        PqcProvider::verify(variant, &pqc_pub, &pqc_tbs, pqc_sig_bytes).ok()?;

        match ciborium::from_reader::<serde_json::Value, _>(&payload[..]) {
            Ok(v) => Some(v),
            Err(_) => {
                // Fallback to plain JSON if it's not CBOR
                serde_json::from_slice(&payload).ok()
            }
        }
    }

    fn encode_map(entries: Vec<(Value, Value)>) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        ciborium::into_writer(&Value::Map(entries), &mut bytes)?;
        Ok(bytes)
    }

    fn build_sig_structure(
        body_protected: &[u8],
        sign_protected: &[u8],
        payload: &[u8],
    ) -> Result<Vec<u8>> {
        let structure = Value::Array(vec![
            Value::Text("Signature".to_string()),
            Value::Bytes(body_protected.to_vec()),
            Value::Bytes(sign_protected.to_vec()),
            Value::Bytes(vec![]), // external_aad
            Value::Bytes(payload.to_vec()),
        ]);
        let mut v = Vec::new();
        ciborium::into_writer(&structure, &mut v)?;
        Ok(v)
    }
}
