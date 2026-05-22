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

/// Type-safe status for audit log entries, replacing free-form strings.
///
/// This enum captures all known states that an audit entry can be in,
/// ensuring that downstream consumers (log rotation, verification, dashboards)
/// can pattern-match on well-defined variants instead of comparing raw
/// strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum AuditStatus {
    /// The tool call completed successfully.
    Success,
    /// The tool call failed with an error message.
    Failed(String),
    /// PQC encryption failed in high-security mode; entry stored with redacted args.
    PqcEncryptionFailed(String),
    /// PQC signing failed in high-security mode; entry stored without signature.
    IntegrityFailure(String),
    /// Entry was stored without a PQC signature because the private key was unavailable.
    SuccessWithoutSignature,
    /// Log rotation continuity marker (not a real tool call).
    #[doc(hidden)]
    LogRotationMarker,
}

impl std::fmt::Display for AuditStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuditStatus::Success => write!(f, "SUCCESS"),
            AuditStatus::Failed(reason) => write!(f, "FAILED: {}", reason),
            AuditStatus::PqcEncryptionFailed(reason) => {
                write!(f, "FAILED: {}; PQC_ENCRYPTION_FAILED", reason)
            }
            AuditStatus::IntegrityFailure(reason) => {
                write!(f, "INTEGRITY_FAILURE: {}", reason)
            }
            AuditStatus::SuccessWithoutSignature => {
                write!(f, "SUCCESS_WITHOUT_SIGNATURE: PQC private key unavailable")
            }
            AuditStatus::LogRotationMarker => write!(f, "CONTINUITY_MAINTAINED"),
        }
    }
}

impl TryFrom<String> for AuditStatus {
    type Error = String;

    fn try_from(s: String) -> Result<Self, String> {
        if s == "SUCCESS" {
            Ok(AuditStatus::Success)
        } else if let Some(reason) = s.strip_prefix("FAILED: ") {
            // Check for the PQC_ENCRYPTION_FAILED suffix
            if let Some(inner) = reason.strip_suffix("; PQC_ENCRYPTION_FAILED") {
                Ok(AuditStatus::PqcEncryptionFailed(inner.to_string()))
            } else {
                Ok(AuditStatus::Failed(reason.to_string()))
            }
        } else if let Some(reason) = s.strip_prefix("INTEGRITY_FAILURE: ") {
            Ok(AuditStatus::IntegrityFailure(reason.to_string()))
        } else if s.starts_with("SUCCESS_WITHOUT_SIGNATURE") {
            Ok(AuditStatus::SuccessWithoutSignature)
        } else if s == "CONTINUITY_MAINTAINED" {
            Ok(AuditStatus::LogRotationMarker)
        } else {
            // Forward-compatible: unknown statuses are wrapped in Failed
            Ok(AuditStatus::Failed(s))
        }
    }
}

impl From<AuditStatus> for String {
    fn from(status: AuditStatus) -> String {
        status.to_string()
    }
}

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
    pub status: AuditStatus,
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
    let tool_name = params.tool_name.to_string();
    let event_type = params.event_type.to_string();
    let result = log_audit_and_return(params, None);
    if result.is_none() {
        tracing::error!(
            tool = %tool_name,
            event = %event_type,
            "Audit log entry was not persisted — integrity gap"
        );
    }
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
    let mut encryption_failed = false;

    if params.config.security.security_level == crate::config::models::SecurityLevel::High
        && let Ok(pk) = crate::security::identity::IdentityManager::get_kem_public_key()
    {
        match serde_json::to_vec(&params.args) {
            Ok(arg_bytes) => {
                match crate::security::pqc::SecureStorage::encrypt(&arg_bytes, &pk) {
                    Ok(packet) => {
                        final_args =
                            serde_json::to_value(packet).unwrap_or_else(|_| params.args.clone());
                        pqc_encrypted = true;
                    }
                    Err(e) => {
                        // Encryption failure in high-security mode is a critical integrity event.
                        // Rather than silently dropping the audit entry (which creates a forensic
                        // gap an attacker could exploit), we record the failure and store redacted
                        // args so the event is still traceable.
                        tracing::error!(
                            tool = params.tool_name,
                            error = %e,
                            "PQC audit encryption failed in high-security mode; storing redacted entry"
                        );
                        final_args = serde_json::json!({
                            "pqc_encryption": "FAILED",
                            "error": format!("{}", e),
                            "args_redacted": true
                        });
                        encryption_failed = true;
                    }
                }
            }
            Err(e) => {
                tracing::error!(
                    tool = params.tool_name,
                    error = %e,
                    "Failed to serialize args for PQC encryption; storing redacted entry"
                );
                final_args = serde_json::json!({
                    "pqc_encryption": "FAILED",
                    "error": format!("Arg serialization failed: {}", e),
                    "args_redacted": true
                });
                encryption_failed = true;
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
        status: {
            if encryption_failed {
                match params.error {
                    None => AuditStatus::PqcEncryptionFailed(String::new()),
                    Some(e) => AuditStatus::PqcEncryptionFailed(e.to_string()),
                }
            } else {
                match params.error {
                    None => AuditStatus::Success,
                    Some(e) => AuditStatus::Failed(e.to_string()),
                }
            }
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

    // Calculate hash over COMPLETE data before truncation for integrity.
    // If serialization fails, the hash chain must still be maintained —
    // we generate a deterministic fallback hash from the trace_id and
    // timestamp to avoid producing a predictable (empty-string) hash.
    let entry_json = match serde_json::to_string(&log_entry) {
        Ok(json) => json,
        Err(e) => {
            tracing::error!(
                tool = params.tool_name,
                error = %e,
                "CRITICAL: Audit entry serialization failed; using fallback hash"
            );
            log_entry.status =
                AuditStatus::IntegrityFailure(format!("Entry serialization failed: {}", e));
            format!(
                "{{\"fallback\": true, \"trace_id\": \"{}\", \"timestamp\": \"{}\"}}",
                log_entry.trace_id, log_entry.timestamp
            )
        }
    };
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
                // In high-security mode, a signing failure is a critical event,
                // but we still persist the entry (with redacted args if needed)
                // so the event is traceable in forensic analysis.
                tracing::error!(
                    tool = params.tool_name,
                    error = %e,
                    "PQC Sign failed in high-security mode; storing entry without signature"
                );
                log_entry.status = AuditStatus::IntegrityFailure(format!("PQC Sign failed: {}", e));
                // Do NOT return None — persist the entry for forensic traceability.
            }
        }
    } else if params.config.security.security_level == crate::config::models::SecurityLevel::High {
        let msg = "PQC Private Key unavailable in high-security mode.";
        tracing::error!(
            tool = params.tool_name,
            "{}; storing entry without signature",
            msg
        );
        log_entry.status = AuditStatus::IntegrityFailure(msg.to_string());
        // Do NOT return None — persist for forensic traceability.
    } else {
        tracing::warn!(
            tool = params.tool_name,
            "PQC private key unavailable; audit entry will be stored without signature"
        );
        log_entry.status = AuditStatus::SuccessWithoutSignature;
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
            if params.config.security.security_level == crate::config::models::SecurityLevel::High {
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
                if params.config.security.security_level
                    == crate::config::models::SecurityLevel::High
                {
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

    // Update the head-pointer cache so the next session can find this
    // hash in O(1) without scanning the entire log file.
    write_head_cache(&log_entry.hash);

    Some(log_entry)
}

/// Sentinel hash used when no previous log entry exists (genesis of the hash chain).
/// This is SHA-256 of the empty string — a well-known, deterministic constant
/// indistinguishable from a real chain hash.
///
/// # Important
/// This constant is **tightly coupled to the hash function used throughout the
/// audit chain** (`SHA-256`).  If the hash algorithm is ever changed (e.g.
/// upgrading to SHA-512 or a post-quantum hash), this sentinel **must** be
/// updated to the corresponding hash of the empty string for the new algorithm,
/// otherwise all existing log chains will fail verification.
const GENESIS_HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// Read the head-pointer cache for O(1) last-hash lookup.
///
/// Returns `Some(hash)` if the cache exists and contains a valid hex string,
/// or `None` if the cache is absent, stale, or corrupt (in which case the
/// caller falls back to a full-file scan).
fn read_head_cache() -> Option<String> {
    let cache_path = crate::consts::audit_head_cache_path();
    if !cache_path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(&cache_path).ok()?;
    let line = content.lines().next()?;
    let hash = line.trim();
    // Sanity check: SHA-256 hex must be exactly 64 hex chars
    if hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(hash.to_string())
    } else {
        None
    }
}

/// Write (or update) the head-pointer cache with the hash of the newest entry.
fn write_head_cache(hash: &str) {
    let cache_path = crate::consts::audit_head_cache_path();
    if let Some(parent) = cache_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Write atomically via temp file + rename to avoid partial-write races.
    let tmp_path = cache_path.with_extension("cache.tmp");
    if let Ok(mut f) = std::fs::File::create(&tmp_path) {
        use std::io::Write;
        if writeln!(f, "{}", hash).is_ok() {
            drop(f);
            let _ = std::fs::rename(&tmp_path, &cache_path);
            return;
        }
        drop(f);
    }
    // Non-atomic fallback: direct write (better than losing the cache).
    let _ = std::fs::write(&cache_path, format!("{}\n", hash));
}

fn get_last_log_hash(path: &Path) -> String {
    // 1. Try the head-pointer cache first (O(1) path).
    if let Some(cached_hash) = read_head_cache() {
        // Verify the cache is not stale: if the log file was truncated or
        // rotated since the cache was written, we must fall back to a scan.
        // A simple heuristic: if the log file doesn't exist at all, return genesis.
        if !path.exists() {
            return GENESIS_HASH.to_string();
        }
        // If the log file exists and is non-empty, trust the cache because
        // `write_head_cache` is always called after a successful append.
        // If the log was externally truncated (manual edit), the cache will
        // be overwritten on the next write.
        if let Ok(metadata) = std::fs::metadata(path)
            && metadata.len() > 0
        {
            return cached_hash;
        }
        // Fall through to full scan if the log is empty but cache exists.
    }

    // 2. Fallback: full backward scan (original logic, now only used as
    //    a recovery path when the cache is absent or the log is empty).
    if !path.exists() {
        // No log file exists yet — create the cache with the genesis hash.
        write_head_cache(GENESIS_HASH);
        return GENESIS_HASH.to_string();
    }

    if let Ok(mut file) = std::fs::File::open(path) {
        let size = file.metadata().map(|m| m.len()).unwrap_or(0);
        if size == 0 {
            write_head_cache(GENESIS_HASH);
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
                    // Cache the found hash for future O(1) lookups.
                    write_head_cache(hash);
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
    write_head_cache(GENESIS_HASH);
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
        if let Err(e) = writeln!(temp_file, "{}", continuity_marker) {
            tracing::error!(path = %temp_path.display(), error = %e, "Failed to write continuity marker during log rotation");
        }

        for l in all_lines.into_iter().skip(skip_count) {
            if let Err(e) = writeln!(temp_file, "{}", l) {
                tracing::error!(path = %temp_path.display(), error = %e, "Failed to write audit entry during log rotation");
            }
        }

        // Finalize by renaming
        drop(temp_file);
        if let Err(e) = fs::rename(&temp_path, path) {
            tracing::error!(
                src = %temp_path.display(),
                dst = %path.display(),
                error = %e,
                "CRITICAL: Failed to rotate audit log — old data retained, new data may be lost"
            );
        } else {
            // Rotation succeeded — invalidate the head cache so the next read
            // falls back to a full scan and rebuilds the cache with the new
            // last-entry hash from the rotated file.
            let cache_path = crate::consts::audit_head_cache_path();
            let _ = std::fs::remove_file(&cache_path);
        }
    } else {
        tracing::error!(path = %temp_path.display(), "Failed to create temp file for audit log rotation");
    }
}
