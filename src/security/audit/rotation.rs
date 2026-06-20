use crate::consts::audit_head_cache_path;
use jiff::Timestamp;
use std::io::Write;
use uuid::Uuid;

/// Sentinel hash used when no previous log entry exists (genesis of the hash chain).
/// This is SHA-256 of the empty string.
const GENESIS_HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// Trim the audit log file to keep only the most recent `max_lines` entries.
/// Inserts a continuity marker at the beginning of the trimmed file.
pub fn trim_log_file(path: &std::path::Path, max_lines: usize) {
    if !path.exists() {
        return;
    }

    use std::io::BufRead;

    let all_lines: Vec<String> = {
        let file = match std::fs::File::open(path) {
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

    // Capture the hash of the last line that will be discarded
    let last_removed_hash = all_lines
        .get(skip_count - 1)
        .and_then(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .and_then(|v| {
            v.get("hash")
                .and_then(|h| h.as_str())
                .map(std::string::ToString::to_string)
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
    if let Ok(mut temp_file) = std::fs::File::create(&temp_path) {
        let continuity_marker = serde_json::json!({
            "timestamp": Timestamp::now().to_string(),
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
        if let Err(e) = writeln!(temp_file, "{continuity_marker}") {
            tracing::error!(path = %temp_path.display(), error = %e, "Failed to write continuity marker during log rotation");
        }

        for l in all_lines.into_iter().skip(skip_count) {
            if let Err(e) = writeln!(temp_file, "{l}") {
                tracing::error!(path = %temp_path.display(), error = %e, "Failed to write audit entry during log rotation");
            }
        }

        // Finalize by renaming
        drop(temp_file);
        if let Err(e) = std::fs::rename(&temp_path, path) {
            tracing::error!(
                src = %temp_path.display(),
                dst = %path.display(),
                error = %e,
                "CRITICAL: Failed to rotate audit log - old data retained, new data may be lost"
            );
        } else {
            // Rotation succeeded - invalidate the head cache
            let cache_path = audit_head_cache_path();
            if let Err(e) = std::fs::remove_file(&cache_path) {
                tracing::debug!(
                    path = %cache_path.display(),
                    error = %e,
                    "Failed to remove stale head cache after rotation (will be rebuilt on next access)"
                );
            }
        }
    } else {
        tracing::error!(path = %temp_path.display(), "Failed to create temp file for audit log rotation");
    }
}
