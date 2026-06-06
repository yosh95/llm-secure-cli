/// Sentinel hash used when no previous log entry exists (genesis of the hash chain).
/// This is SHA-256 of the empty string.
const GENESIS_HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// Read the head-pointer cache for O(1) last-hash lookup.
fn read_head_cache() -> Option<String> {
    let cache_path = crate::consts::audit_head_cache_path();
    if !cache_path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(&cache_path).ok()?;
    let line = content.lines().next()?;
    let hash = line.trim();
    if hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(hash.to_string())
    } else {
        None
    }
}

/// Write (or update) the head-pointer cache with the hash of the newest entry.
pub fn write_head_cache(hash: &str) {
    let cache_path = crate::consts::audit_head_cache_path();
    if let Some(parent) = cache_path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        tracing::warn!(
            path = %parent.display(),
            error = %e,
            "Failed to create head cache directory"
        );
    }

    // Atomic write: write to temp file, flush, then rename.
    // No fallback to direct write — a partial/direct write could corrupt the cache,
    // causing get_last_log_hash to return stale hashes and breaking the chain.
    // If rename fails, the next session will rebuild the cache from the log file.
    let tmp_path = cache_path.with_extension("cache.tmp");
    let mut f = match std::fs::File::create(&tmp_path) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(
                path = %tmp_path.display(),
                error = %e,
                "Failed to create head cache temp file (non-critical: will rebuild)"
            );
            return;
        }
    };

    use std::io::Write;
    if let Err(e) = writeln!(f, "{hash}") {
        tracing::warn!(
            path = %tmp_path.display(),
            error = %e,
            "Failed to write head cache temp file (non-critical: will rebuild)"
        );
        let _ = std::fs::remove_file(&tmp_path);
        return;
    }
    // Explicitly flush before rename to ensure all data is on disk.
    if let Err(e) = f.flush() {
        tracing::warn!(
            error = %e,
            "Failed to flush head cache temp file (non-critical: will rebuild)"
        );
        let _ = std::fs::remove_file(&tmp_path);
        return;
    }
    drop(f);

    if let Err(e) = std::fs::rename(&tmp_path, &cache_path) {
        tracing::warn!(
            src = %tmp_path.display(),
            dst = %cache_path.display(),
            error = %e,
            "Failed to atomically rename head cache (non-critical: next session will rebuild)"
        );
        // Clean up temp file on failure.
        let _ = std::fs::remove_file(&tmp_path);
    }
}

pub fn get_last_log_hash(path: &std::path::Path) -> String {
    use std::io::{Read, Seek, SeekFrom};

    // 1. Try the head-pointer cache first (O(1) path).
    if let Some(cached_hash) = read_head_cache() {
        if !path.exists() {
            return GENESIS_HASH.to_string();
        }
        if let Ok(metadata) = std::fs::metadata(path)
            && metadata.len() > 0
        {
            return cached_hash;
        }
    }

    // 2. Fallback: full backward scan
    if !path.exists() {
        write_head_cache(GENESIS_HASH);
        return GENESIS_HASH.to_string();
    }

    if let Ok(mut file) = std::fs::File::open(path) {
        let size = file.metadata().map_or(0, |m| m.len());
        if size == 0 {
            write_head_cache(GENESIS_HASH);
            return GENESIS_HASH.to_string();
        }

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

            chunk.extend_from_slice(&buffer);
            buffer = chunk;

            let content = String::from_utf8_lossy(&buffer);
            let mut lines: Vec<&str> = content.split('\n').collect();

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
                    write_head_cache(hash);
                    return hash.to_string();
                }
            }

            if size - pos > 1024 * 1024 {
                break;
            }
        }
    }

    tracing::warn!(
        path = %path.display(),
        "Could not find last hash in audit log; chain may be broken - starting new genesis"
    );
    write_head_cache(GENESIS_HASH);
    GENESIS_HASH.to_string()
}
