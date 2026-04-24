use crate::consts::AUDIT_LOG_PATH;
use crate::security::audit::AuditEntry;
use crate::security::identity::IdentityManager;
use crate::security::merkle::MerkleTree;
use crate::security::pqc::{MldsaVariant, PqcProvider};
use anyhow::Result;
use base64::{Engine as _, engine::general_purpose};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Digest;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

static ANCHOR_DIR: Lazy<PathBuf> = Lazy::new(|| AUDIT_LOG_PATH.parent().unwrap().join("anchors"));

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SessionAnchor {
    pub trace_id: String,
    pub merkle_root: String,
    pub entry_count: usize,
    pub first_entry_hash: String,
    pub last_entry_hash: String,
    pub timestamp: Option<Value>,
    pub anchored_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pqc_signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pqc_algorithm: Option<String>,
}

pub struct SessionAnchorManager;

impl SessionAnchorManager {
    pub fn get_session_entries(trace_id: &str) -> Vec<Value> {
        let mut entries = Vec::new();
        let log_path = &*AUDIT_LOG_PATH;

        let mut log_files = Vec::new();

        if let Ok(read_dir) = fs::read_dir(log_path.parent().unwrap()) {
            for entry in read_dir.flatten() {
                let name = entry.file_name();
                let name_str = name.to_str().unwrap();
                if name_str.starts_with(log_path.file_name().unwrap().to_str().unwrap())
                    && name_str.contains(".archive.")
                {
                    log_files.push(entry.path());
                }
            }
        }
        log_files.sort();
        log_files.push(log_path.to_path_buf());

        for path in log_files {
            if !path.exists() {
                continue;
            }
            if let Ok(file) = File::open(path) {
                let reader = BufReader::new(file);
                for line in reader.lines().map_while(Result::ok) {
                    if let Ok(entry) = serde_json::from_str::<Value>(&line)
                        && entry.get("trace_id").and_then(|v| v.as_str()) == Some(trace_id)
                    {
                        entries.push(entry);
                    }
                }
            }
        }
        entries
    }

    pub fn create_anchor(trace_id: &str, entries: Option<Vec<Value>>) -> Result<Option<String>> {
        let entries = if let Some(e) = entries {
            e
        } else {
            Self::get_session_entries(trace_id)
        };

        if entries.is_empty() {
            return Ok(None);
        }

        let leaf_hashes: Vec<String> = entries
            .iter()
            .filter_map(|e| {
                e.get("hash")
                    .and_then(|h| h.as_str())
                    .map(|s| s.to_string())
            })
            .collect();

        if leaf_hashes.is_empty() {
            return Ok(None);
        }

        let tree = MerkleTree::new(leaf_hashes.clone());
        let root_hex = tree.root_hex.clone();

        let mtime = AUDIT_LOG_PATH
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs());

        let mut anchor = SessionAnchor {
            trace_id: trace_id.to_string(),
            merkle_root: root_hex.clone(),
            entry_count: entries.len(),
            first_entry_hash: leaf_hashes[0].clone(),
            last_entry_hash: leaf_hashes.last().unwrap().clone(),
            timestamp: entries.last().and_then(|e| e.get("timestamp").cloned()),
            anchored_at: mtime,
            pqc_signature: None,
            pqc_algorithm: None,
        };

        // Sign with PQC
        let variant = MldsaVariant::Mldsa65;
        if let Ok(sk) = IdentityManager::get_pqc_private_key(variant) {
            let message = serde_json::to_string(&anchor)?;
            let sig = PqcProvider::sign_mldsa(message.as_bytes(), &sk, variant)?;
            anchor.pqc_signature = Some(general_purpose::STANDARD.encode(sig));
            anchor.pqc_algorithm = Some(variant.to_str().to_string());
        }

        fs::create_dir_all(&*ANCHOR_DIR)?;
        let anchor_path = ANCHOR_DIR.join(format!("{}.anchor.json", trace_id));
        let f = File::create(anchor_path)?;
        serde_json::to_writer_pretty(f, &anchor)?;

        Ok(Some(root_hex))
    }

    pub fn verify_session(trace_id: &str) -> Result<bool> {
        let anchor_path = ANCHOR_DIR.join(format!("{}.anchor.json", trace_id));
        if !anchor_path.exists() {
            return Ok(false);
        }

        let f = File::open(anchor_path)?;
        let anchor: SessionAnchor = serde_json::from_reader(f)?;

        // 1. Verify PQC Signature
        if let (Some(sig_b64), Some(algo_str)) = (&anchor.pqc_signature, &anchor.pqc_algorithm) {
            use std::str::FromStr;
            let variant = MldsaVariant::from_str(algo_str).unwrap_or(MldsaVariant::Mldsa65);
            let pk = IdentityManager::get_pqc_public_key(variant)?;

            let mut anchor_copy = anchor.clone();
            anchor_copy.pqc_signature = None;
            anchor_copy.pqc_algorithm = None;
            let message = serde_json::to_string(&anchor_copy)?;

            let sig = general_purpose::STANDARD.decode(sig_b64)?;
            if !PqcProvider::verify_mldsa(message.as_bytes(), &sig, &pk, variant) {
                return Ok(false);
            }
        }

        // 2. Verify Merkle Root
        let entries = Self::get_session_entries(trace_id);
        if entries.len() != anchor.entry_count {
            return Ok(false);
        }

        let mut leaf_hashes = Vec::new();
        for entry in entries {
            let provided_hash = entry
                .get("hash")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing hash"))?
                .to_string();

            // Re-serialize exactly as log_audit does (via AuditEntry struct)
            let mut audit_entry: AuditEntry = serde_json::from_value(entry)?;
            audit_entry.hash = String::new();
            audit_entry.pqc_signature = None;
            audit_entry.pqc_algorithm = None;

            let entry_str = serde_json::to_string(&audit_entry)?;
            let mut hasher = sha2::Sha256::new();
            hasher.update(entry_str.as_bytes());
            let actual_hash = hex::encode(hasher.finalize());

            if provided_hash != actual_hash {
                return Ok(false);
            }
            leaf_hashes.push(provided_hash);
        }

        let tree = MerkleTree::new(leaf_hashes);
        Ok(tree.root_hex == anchor.merkle_root)
    }
}
