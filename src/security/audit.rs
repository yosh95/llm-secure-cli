use crate::consts::audit_log_path;
use crate::security::pqc::ResponseSigner;
use chrono::Utc;
use hostname;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Clone)]
pub struct AuditEntry {
    pub timestamp: String,
    pub trace_id: String,
    pub subject: String,
    pub audience: String,
    pub model: String,
    pub provider: String,
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
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub cli_version: String,
}

pub struct AuditParams<'a> {
    pub event_type: &'a str,
    pub tool_name: &'a str,
    pub args: serde_json::Value,
    pub output: Option<&'a str>,
    pub exit_code: Option<i32>,
    pub error: Option<&'a str>,
    pub context: Option<&'a serde_json::Value>,
    pub config: &'a crate::config::models::AppConfig,
}

pub struct AuditParamsBuilder<'a> {
    params: AuditParams<'a>,
}

impl<'a> AuditParamsBuilder<'a> {
    pub fn new(
        event_type: &'a str,
        tool_name: &'a str,
        config: &'a crate::config::models::AppConfig,
    ) -> Self {
        Self {
            params: AuditParams {
                event_type,
                tool_name,
                args: serde_json::json!({}),
                output: None,
                exit_code: None,
                error: None,
                context: None,
                config,
            },
        }
    }

    pub fn args(mut self, args: serde_json::Value) -> Self {
        self.params.args = args;
        self
    }

    pub fn output(mut self, output: &'a str) -> Self {
        self.params.output = Some(output);
        self
    }

    pub fn exit_code(mut self, code: i32) -> Self {
        self.params.exit_code = Some(code);
        self
    }

    pub fn error(mut self, error: &'a str) -> Self {
        self.params.error = Some(error);
        self
    }

    pub fn context(mut self, context: &'a serde_json::Value) -> Self {
        self.params.context = Some(context);
        self
    }

    pub fn log(self) {
        log_audit(self.params);
    }

    pub fn log_and_return(self, log_path: Option<&std::path::Path>) -> Option<AuditEntry> {
        log_audit_and_return(self.params, log_path)
    }
}

impl<'a> AuditParams<'a> {
    pub fn builder(
        event_type: &'a str,
        tool_name: &'a str,
        config: &'a crate::config::models::AppConfig,
    ) -> AuditParamsBuilder<'a> {
        AuditParamsBuilder::new(event_type, tool_name, config)
    }
}

pub fn log_audit(params: AuditParams) {
    let _ = log_audit_and_return(params, None);
}

pub fn log_audit_and_return(params: AuditParams, log_path: Option<&Path>) -> Option<AuditEntry> {
    let a_path = audit_log_path();
    let default_path = &a_path;
    let path = log_path.unwrap_or(default_path);
    let max_lines = params.config.general.max_audit_log_lines;

    if let Some(parent) = path.parent()
        && !parent.exists()
    {
        let _ = fs::create_dir_all(parent);
    }

    let timestamp = Utc::now().to_rfc3339();
    let empty_map = serde_json::Map::new();
    let ctx = params
        .context
        .and_then(|c| c.as_object())
        .unwrap_or(&empty_map);

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
    let provider = ctx
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("-")
        .to_string();

    let hostname = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string());
    let os = std::env::consts::OS.to_string();
    let arch = std::env::consts::ARCH.to_string();
    let cli_version = env!("CARGO_PKG_VERSION").to_string();

    let prev_hash = get_last_log_hash(path);

    // Hybrid Encryption for high-risk data
    let mut pqc_encrypted = false;
    let mut final_args = params.args.clone();

    if params.config.security.security_level == "high"
        && let Ok(pk) = crate::security::identity::IdentityManager::get_kem_public_key()
    {
        let arg_bytes = serde_json::to_vec(&params.args).unwrap_or_default();
        match crate::security::pqc::SecureStorage::encrypt(&arg_bytes, &pk) {
            Ok(packet) => {
                final_args = serde_json::to_value(packet).unwrap_or_else(|_| params.args.clone());
                pqc_encrypted = true;
            }
            Err(e) => {
                // Encryption failure in high-security mode is a critical integrity event.
                // Block the audit write entirely rather than persisting plaintext args.
                if params.config.security.security_level == "high" {
                    crate::cli::ui::report_error(&format!(
                        "CRITICAL SECURITY ERROR: PQC audit encryption failed in high-security mode: {}",
                        e
                    ));
                    return None;
                }
                // In standard mode, log a warning and fall back to plaintext args.
                tracing::warn!(
                    tool = params.tool_name,
                    error = %e,
                    "PQC audit encryption failed; storing plaintext args (standard mode)"
                );
            }
        }
    }

    let mut log_entry = AuditEntry {
        timestamp,
        trace_id,
        subject,
        audience,
        model,
        provider,
        event_type: params.event_type.to_string(),
        tool: params.tool_name.to_string(),
        args: final_args,
        pqc_confidential: pqc_encrypted,
        output: params.output.map(|s| s.to_string()),
        status: match params.error {
            None => "SUCCESS".to_string(),
            Some(e) => format!("FAILED: {}", e),
        },
        exit_code: params.exit_code,
        prev_hash,
        hash: String::new(),
        pqc_signature: None,
        pqc_algorithm: None,
        hostname,
        os,
        arch,
        cli_version,
    };

    // Calculate hash over COMPLETE data before truncation for integrity
    let entry_json = serde_json::to_string(&log_entry).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(entry_json.as_bytes());
    log_entry.hash = crate::utils::hex_encode(hasher.finalize());

    // PQC Signing
    let variant = crate::security::pqc::PQCAgilityManager::get_required_level(
        params.config,
        params.tool_name,
        Some(&params.args),
    );
    if let Ok(sk) = crate::security::identity::IdentityManager::get_pqc_private_key(variant) {
        match ResponseSigner::sign_response(&log_entry.hash, &log_entry.trace_id, &sk, variant) {
            Ok(signed) => {
                log_entry.pqc_signature = signed
                    .get("pqc_signature")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                log_entry.pqc_algorithm = signed
                    .get("algorithm")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
            }
            Err(e) => {
                if params.config.security.security_level == "high" {
                    crate::cli::ui::report_error(&format!(
                        "CRITICAL SECURITY ERROR: PQC Sign failed in high-security mode: {}",
                        e
                    ));
                    log_entry.status = format!("INTEGRITY_FAILURE: PQC Sign failed: {}", e);
                    return None; // Integrity failure, block audit write
                } else {
                    log_entry.status = format!("SUCCESS_WITHOUT_SIGNATURE: {}", e);
                }
            }
        }
    } else if params.config.security.security_level == "high" {
        let msg = "CRITICAL SECURITY ERROR: PQC Private Key unavailable in high-security mode.";
        crate::cli::ui::report_error(msg);
        log_entry.status = format!("INTEGRITY_FAILURE: {}", msg);
        return None;
    } else {
        tracing::warn!(
            tool = params.tool_name,
            "PQC private key unavailable; audit entry will be stored without signature"
        );
        log_entry.status = "SUCCESS_WITHOUT_SIGNATURE: PQC private key unavailable".to_string();
    }

    // Now truncate the output only for storage efficiency
    if let Some(ref mut out) = log_entry.output
        && out.len() > 1024
    {
        let mut end = 1024;
        while !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
        out.push_str("...[TRUNCATED]");
    }

    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    match options.open(path) {
        Err(e) => {
            tracing::error!(path = %path.display(), error = %e, "audit log open failed");
            if params.config.security.security_level == "high" {
                crate::cli::ui::report_error(&format!("CRITICAL: Audit log unavailable: {}", e));
            }
            return None;
        }
        Ok(mut file) => {
            let line = match serde_json::to_string(&log_entry) {
                Ok(l) => l,
                Err(e) => {
                    tracing::error!(error = %e, "audit log serialization failed");
                    return None;
                }
            };
            if let Err(e) = writeln!(file, "{}", line) {
                tracing::error!(path = %path.display(), error = %e, "audit log write failed");
                if params.config.security.security_level == "high" {
                    crate::cli::ui::report_error(&format!(
                        "CRITICAL: Audit log write failed: {}",
                        e
                    ));
                }
                return None;
            }
        }
    }

    if let Ok(metadata) = fs::metadata(path) {
        let estimated_line_count = metadata.len() / 2000;
        if estimated_line_count > (max_lines as u64 * 11 / 10) {
            trim_log_file(path, max_lines);
        }
    }

    Some(log_entry)
}

/// Sentinel hash used when no previous log entry exists (genesis of the hash chain).
/// This is SHA-256 of the empty string — a well-known, deterministic constant
/// indistinguishable from a real chain hash.
const GENESIS_HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

fn get_last_log_hash(path: &Path) -> String {
    if !path.exists() {
        return GENESIS_HASH.to_string();
    }

    if let Ok(mut file) = fs::File::open(path) {
        let size = file.metadata().map(|m| m.len()).unwrap_or(0);
        if size == 0 {
            return GENESIS_HASH.to_string();
        }

        // Scan backwards from end of file to find the last valid JSON entry
        let mut pos = size;
        let mut buffer = Vec::new();
        let chunk_size = 4096;

        while pos > 0 {
            let to_read = std::cmp::min(pos, chunk_size as u64);
            pos -= to_read;

            let mut chunk = vec![0; to_read as usize];
            if i64::try_from(pos).is_err()
                || file.seek(SeekFrom::Start(pos)).is_err()
                || file.read_exact(&mut chunk).is_err()
            {
                break;
            }

            // Prepend chunk to what we've collected so far
            chunk.extend_from_slice(&buffer);
            buffer = chunk;

            // Search for full lines from the end
            let content = String::from_utf8_lossy(&buffer);
            let mut lines: Vec<&str> = content.split('\n').collect();

            // The last fragment after the last '\n' might be empty or incomplete
            if content.ends_with('\n') {
                lines.pop();
            }

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

            // Limit backward search to 1MB to avoid performance issues on corrupt logs
            if size - pos > 1024 * 1024 {
                break;
            }
        }
    }

    // If we couldn't find the last hash (corrupt log, empty, etc.),
    // return the sentinel genesis hash to indicate the chain is broken.
    tracing::warn!(
        path = %path.display(),
        "Could not find last hash in audit log; chain may be broken — starting new genesis"
    );
    GENESIS_HASH.to_string()
}

fn trim_log_file(path: &std::path::Path, max_lines: usize) {
    if !path.exists() {
        return;
    }

    use std::io::BufRead;

    // Single-pass: read all lines into memory at once.
    // This avoids the TOCTOU race that arose from opening the file twice
    // (count pass → skip pass), where a concurrent writer could append
    // lines between the two opens and shift the skip index.
    let all_lines: Vec<String> = {
        let file = match fs::File::open(path) {
            Ok(f) => f,
            Err(_) => return,
        };
        std::io::BufReader::new(file)
            .lines()
            .map_while(Result::ok)
            .collect()
    };

    let total_lines = all_lines.len();
    if total_lines <= max_lines {
        return;
    }

    let skip_count = total_lines - max_lines;

    // Capture the hash of the last line that will be discarded so the
    // continuity marker can chain the hash log correctly.
    let last_removed_hash = all_lines
        .get(skip_count - 1)
        .and_then(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .and_then(|v| {
            v.get("hash")
                .and_then(|h| h.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| GENESIS_HASH.to_string());

    let hostname = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string());
    let os = std::env::consts::OS.to_string();
    let arch = std::env::consts::ARCH.to_string();
    let cli_version = env!("CARGO_PKG_VERSION").to_string();

    // Write the trimmed content to a temp file, then atomically rename.
    let temp_path = path.with_extension("tmp");
    if let Ok(mut temp_file) = fs::File::create(&temp_path) {
        let continuity_marker = serde_json::json!({
            "timestamp": Utc::now().to_rfc3339(),
            "trace_id": "rotation",
            "subject": "system",
            "audience": "audit",
            "model": "-",
            "provider": "-",
            "event_type": "LOG_ROTATION_MARKER",
            "tool": "system",
            "args": {},
            "pqc_confidential": false,
            "prev_hash": last_removed_hash,
            "hash": format!("ROTATION-NONCE-{}", Uuid::new_v4()),
            "status": "CONTINUITY_MAINTAINED",
            "hostname": hostname,
            "os": os,
            "arch": arch,
            "cli_version": cli_version
        });
        let _ = writeln!(temp_file, "{}", continuity_marker);

        for l in all_lines.into_iter().skip(skip_count) {
            let _ = writeln!(temp_file, "{}", l);
        }

        // Finalize by renaming
        drop(temp_file);
        let _ = fs::rename(&temp_path, path);
    }
}
