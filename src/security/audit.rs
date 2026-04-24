use crate::config::CONFIG_MANAGER;
use crate::consts::AUDIT_LOG_PATH;
use crate::security::pqc::ResponseSigner;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AuditEntry {
    pub timestamp: String,
    pub trace_id: String,
    pub subject: String,
    pub audience: String,
    pub model: String,
    pub event_type: String,
    pub tool: String,
    pub args: serde_json::Value,
    pub pqc_confidential: bool,
    pub output: Option<String>,
    pub status: String,
    pub exit_code: Option<i32>,
    pub prev_hash: String,
    pub hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pqc_signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pqc_algorithm: Option<String>,
}

pub fn log_audit(
    event_type: &str,
    tool_name: &str,
    args: serde_json::Value,
    output: Option<&str>,
    exit_code: Option<i32>,
    error: Option<&str>,
    context: Option<&serde_json::Value>,
) {
    let _ = log_audit_and_return(
        event_type, tool_name, args, output, exit_code, error, context,
    );
}

pub fn log_audit_and_return(
    event_type: &str,
    tool_name: &str,
    args: serde_json::Value,
    output: Option<&str>,
    exit_code: Option<i32>,
    error: Option<&str>,
    context: Option<&serde_json::Value>,
) -> Option<AuditEntry> {
    let path = &*AUDIT_LOG_PATH;
    let config = CONFIG_MANAGER.get_config();
    let max_lines = config.general.max_audit_log_lines;

    // Ensure directory exists
    if let Some(parent) = path.parent()
        && !parent.exists()
    {
        let _ = fs::create_dir_all(parent);
    }

    let timestamp = Utc::now().to_rfc3339();
    let empty_map = serde_json::Map::new();
    let ctx = context.and_then(|c| c.as_object()).unwrap_or(&empty_map);

    let trace_id = ctx
        .get("trace_id")
        .and_then(|v| v.as_str())
        .unwrap_or("-")
        .to_string();
    let subject = ctx
        .get("user_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let audience = ctx
        .get("audience")
        .and_then(|v| v.as_str())
        .unwrap_or("-")
        .to_string();
    let model = ctx
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("-")
        .to_string();

    let prev_hash = get_last_log_hash(path);

    // Hybrid Encryption for high-risk data (ML-KEM + AES-256-GCM)
    let mut pqc_encrypted = false;
    let mut final_args = args.clone();

    if config.security.security_level == "high"
        && let Ok(pk) = crate::security::identity::IdentityManager::get_kem_public_key()
    {
        let arg_bytes = serde_json::to_vec(&args).unwrap_or_default();
        let packet = crate::security::pqc::SecureStorage::encrypt(&arg_bytes, &pk);
        final_args = serde_json::to_value(packet).unwrap_or(args);
        pqc_encrypted = true;
    }

    let mut log_entry = AuditEntry {
        timestamp,
        trace_id,
        subject,
        audience,
        model,
        event_type: event_type.to_string(),
        tool: tool_name.to_string(),
        args: final_args,
        pqc_confidential: pqc_encrypted,
        output: output.map(|s| {
            if s.len() > 256 {
                let mut end = 256;
                while !s.is_char_boundary(end) {
                    end -= 1;
                }
                format!("{}...", &s[..end])
            } else {
                s.to_string()
            }
        }),
        status: match error {
            None => "SUCCESS".to_string(),
            Some(e) => format!("FAILED: {}", e),
        },
        exit_code,
        prev_hash,
        hash: String::new(),
        pqc_signature: None,
        pqc_algorithm: None,
    };

    // Calculate hash (Hashed Chain)
    let entry_json = serde_json::to_string(&log_entry).unwrap();
    let mut hasher = Sha256::new();
    hasher.update(entry_json.as_bytes());
    log_entry.hash = hex::encode(hasher.finalize());

    // PQC Signing (Identity Manager now provides the key)
    let variant = crate::security::pqc::MldsaVariant::Mldsa65;
    if let Ok(sk) = crate::security::identity::IdentityManager::get_pqc_private_key(variant) {
        match ResponseSigner::sign_response(&log_entry.hash, &log_entry.trace_id, &sk, variant) {
            Ok(signed) => {
                log_entry.pqc_signature = Some(signed.pqc_signature);
                log_entry.pqc_algorithm = Some(signed.algorithm);
            }
            Err(e) => {
                eprintln!("[ERROR] Failed to sign audit log entry: {}", e);
            }
        }
    }

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path)
        && let Ok(line) = serde_json::to_string(&log_entry)
    {
        let _ = writeln!(file, "{}", line);
    }

    // Only trim if the file has grown significantly beyond max_lines to avoid excessive I/O
    if let Ok(metadata) = fs::metadata(path) {
        let estimated_line_count = metadata.len() / 1500;
        if estimated_line_count > (max_lines as u64 * 11 / 10) {
            trim_log_file(path, max_lines);
        }
    }

    Some(log_entry)
}

fn get_last_log_hash(path: &Path) -> String {
    if !path.exists() {
        return "0".repeat(64);
    }

    if let Ok(mut file) = fs::File::open(path) {
        let size = file.metadata().map(|m| m.len()).unwrap_or(0);
        if size == 0 {
            return "0".repeat(64);
        }

        // Increase buffer size to handle large PQC signatures and long outputs
        let read_size = std::cmp::min(size, 32768);
        let mut buffer = vec![0; read_size as usize];
        let _ = file.seek(SeekFrom::End(-(read_size as i64)));
        let _ = file.read_exact(&mut buffer);

        let content = String::from_utf8_lossy(&buffer);
        let lines: Vec<&str> = content.split('\n').collect();

        // Iterate backwards to find the last valid JSON entry with a hash
        for line in lines.iter().rev() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(entry) = serde_json::from_str::<serde_json::Value>(trimmed)
                && let Some(hash) = entry.get("hash").and_then(|v| v.as_str())
            {
                return hash.to_string();
            }
        }
    }

    "0".repeat(64)
}

fn trim_log_file(path: &std::path::Path, max_lines: usize) {
    if !path.exists() {
        return;
    }

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };

    let lines: Vec<&str> = content.lines().collect();
    if lines.len() <= max_lines {
        return;
    }

    // Capture the hash of the last line that WILL be removed to maintain chain evidence
    let last_removed_idx = lines.len() - max_lines - 1;
    let last_removed_hash = serde_json::from_str::<serde_json::Value>(lines[last_removed_idx])
        .ok()
        .and_then(|v| {
            v.get("hash")
                .and_then(|h| h.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "0".repeat(64));

    let kept_lines = &lines[lines.len() - max_lines..];

    if let Ok(mut file) = std::fs::File::create(path) {
        // Add a continuity marker as the first line of the new truncated log
        let continuity_marker = serde_json::json!({
            "timestamp": Utc::now().to_rfc3339(),
            "event_type": "LOG_ROTATION_MARKER",
            "prev_hash": last_removed_hash,
            "hash": "ROTATION-NONCE-".to_string() + &Uuid::new_v4().to_string(),
            "status": "CONTINUITY_MAINTAINED"
        });
        let _ = writeln!(file, "{}", continuity_marker);

        for line in kept_lines {
            let _ = writeln!(file, "{}", line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use std::fs;

    #[test]
    fn test_trim_log_file_and_rotation_marker() {
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("test_audit.jsonl");

        // 1. Create 10 dummy log entries
        let mut entries = Vec::new();
        let mut last_hash = "0".repeat(64);

        for i in 0..10 {
            let hash = format!("hash_{}", i);
            let entry = serde_json::json!({
                "timestamp": i.to_string(),
                "event_type": "test",
                "prev_hash": last_hash,
                "hash": hash,
                "status": "SUCCESS"
            });
            let line = serde_json::to_string(&entry).unwrap();
            last_hash = hash;
            entries.push(line);
        }
        fs::write(&log_path, entries.join("\n") + "\n").unwrap();

        // 2. Trim to 5 lines (execution of rotation)
        // Indices 0-4 should be removed, 5-9 should remain
        trim_log_file(&log_path, 5);

        // 3. Verification
        let content = fs::read_to_string(&log_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();

        // Expected: Marker(1) + Kept lines(5) = 6 lines
        assert_eq!(lines.len(), 6, "Should have marker line plus 5 kept lines");

        // Verify the first line is the marker
        let marker: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(marker["event_type"], "LOG_ROTATION_MARKER");
        
        // Marker's prev_hash should be the hash of the last removed entry ("hash_4")
        assert_eq!(marker["prev_hash"], "hash_4");
        assert!(marker["hash"].as_str().unwrap().starts_with("ROTATION-NONCE-"));

        // Verify the second line is the first kept log ("hash_5")
        let first_kept: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(first_kept["hash"], "hash_5");
        assert_eq!(first_kept["prev_hash"], "hash_4");
    }
}
