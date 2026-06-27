use crate::consts::audit_log_path;
use crate::security::audit::chain::get_last_log_hash;
use crate::security::audit::rotation::trim_log_file;
use crate::security::audit::types::{AuditEntry, AuditParams, AuditStatus};
use jiff::Timestamp;
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

/// Global mutex that serializes the chain-sensitive portion of audit log writes.
///
/// **What runs *under the lock* (must be atomic per entry):**
/// - `get_last_log_hash()` read (prev_hash)
/// - Hash-chain calculation (SHA-256 over the full entry)
/// - PQC signing (chain-dependent, but fast ~1ms)
/// - File append (single `writeln!`)
/// - `trim_log_file()` (races with concurrent appends)
/// - `write_head_cache()` (must reflect the newest entry)
///
/// Without atomicity, concurrent calls (e.g. Verifier Committee members) would
/// read the same `prev_hash`, producing two entries that both chain from the
/// same parent — a **chain fork** that silently corrupts the tamper-evident
/// audit trail.
///
/// **What runs *outside the lock* (chain-independent, can be concurrent):**
/// - PQC encryption of `args` (ML-KEM-1024, CPU-intensive but does not
///   depend on the prev_hash; encrypted payload is attached to the entry
///   metadata, not the chain linkage)
static AUDIT_LOG_MUTEX: Mutex<()> = Mutex::new(());

pub fn log_audit(params: AuditParams) {
    let tool_name = params.tool_name.to_string();
    let event_type = params.event_type.to_string();
    let result = log_audit_and_return(params, None);
    if result.is_none() {
        tracing::error!(
            tool = %tool_name,
            event = %event_type,
            "Audit log entry was not persisted - integrity gap"
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
        && let Err(e) = fs::create_dir_all(parent)
    {
        tracing::error!(
            path = %parent.display(),
            error = %e,
            "CRITICAL: Failed to create audit log directory"
        );
    }

    let timestamp = Timestamp::now().to_string();
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

    // ML-KEM-1024 key encapsulation is CPU-intensive but chain-independent:
    // the encrypted payload is opaque metadata attached to the entry and
    // does not participate in the hash-chain linkage.  Running it here
    // allows concurrent encryption across Verifier Committee members.
    let mut pqc_encrypted = false;
    let mut final_args = params.args.clone();
    let mut encryption_failed = false;

    if let Ok(pk) = crate::security::identity::IdentityManager::get_kem_public_key() {
        match serde_json::to_vec(&params.args) {
            Ok(arg_bytes) => match crate::security::pqc::SecureStorage::encrypt(&arg_bytes, &pk) {
                Ok(packet) => {
                    final_args =
                        serde_json::to_value(packet).unwrap_or_else(|_| params.args.clone());
                    pqc_encrypted = true;
                }
                Err(e) => {
                    tracing::error!(
                        tool = params.tool_name,
                        error = %e,
                        "PQC audit encryption failed; storing redacted entry"
                    );
                    final_args = serde_json::json!({
                        "pqc_encryption": "FAILED",
                        "error": format!("{}", e),
                        "args_redacted": true
                    });
                    encryption_failed = true;
                }
            },
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

    // All of prev_hash read → entry construction → hash calculation →
    // PQC signature → file write → trim → head-cache update must happen
    // atomically.  Otherwise two concurrent verifiers can both read the
    // same prev_hash, producing a chain fork that silently corrupts the
    // tamper-evident audit trail.
    let _lock = match AUDIT_LOG_MUTEX.lock() {
        Ok(guard) => guard,
        Err(e) => {
            tracing::error!("Audit log mutex poisoned: {e}");
            return None;
        }
    };

    // prev_hash MUST be read under the lock so that a concurrent writer
    // cannot slip a new entry between our read and our append.
    let prev_hash = get_last_log_hash(path);

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
        output: params.output.map(std::string::ToString::to_string),
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
    let entry_json = match serde_json::to_string(&log_entry) {
        Ok(json) => json,
        Err(e) => {
            tracing::error!(
                tool = params.tool_name,
                error = %e,
                "CRITICAL: Audit entry serialization failed; using fallback hash"
            );
            log_entry.status =
                AuditStatus::IntegrityFailure(format!("Entry serialization failed: {e}"));
            format!(
                "{{\"fallback\": true, \"trace_id\": \"{}\", \"timestamp\": \"{}\"}}",
                log_entry.trace_id, log_entry.timestamp
            )
        }
    };
    let mut hasher = Sha256::new();
    hasher.update(entry_json.as_bytes());
    log_entry.hash = crate::utils::hex_encode(hasher.finalize());

    // PQC Signing: chain-dependent (signs log_entry.hash), so must run
    // under the lock.  Signature verification is a fixed-cost operation
    // (~1ms for ML-DSA-87), so lock-hold time remains short.
    let variant = crate::security::pqc::get_signature_variant(params.config);
    if let Ok(sk) = crate::security::identity::IdentityManager::get_pqc_private_key(variant) {
        match crate::security::pqc::ResponseSigner::sign_response(
            &log_entry.hash,
            &log_entry.trace_id,
            &sk,
            variant,
        ) {
            Ok(signed) => {
                log_entry.pqc_signature = signed
                    .get("pqc_signature")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string);
                log_entry.pqc_algorithm = signed
                    .get("algorithm")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string);
            }
            Err(e) => {
                tracing::error!(
                    tool = params.tool_name,
                    error = %e,
                    "PQC Sign failed; storing entry without signature"
                );
                log_entry.status = AuditStatus::IntegrityFailure(format!("PQC Sign failed: {e}"));
            }
        }
    } else {
        let msg = "PQC Private Key unavailable.";
        tracing::error!(
            tool = params.tool_name,
            "{}; storing entry without signature",
            msg
        );
        log_entry.status = AuditStatus::IntegrityFailure(msg.to_string());
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
            crate::cli::ui::report::report_error(&format!("CRITICAL: Audit log unavailable: {e}"));
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
            if let Err(e) = writeln!(file, "{line}") {
                tracing::error!(path = %path.display(), error = %e, "audit log write failed");
                crate::cli::ui::report::report_error(&format!(
                    "CRITICAL: Audit log write failed: {e}"
                ));
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

    // Update the head-pointer cache atomically with the append so the
    // next writer's `get_last_log_hash()` sees this entry's hash.
    crate::security::audit::chain::write_head_cache(&log_entry.hash);

    Some(log_entry)
}
