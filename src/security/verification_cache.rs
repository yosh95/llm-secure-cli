use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Cached verification result with TTL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationCacheEntry {
    pub safe: bool,
    pub reason: String,
    pub timestamp: u64, // Unix epoch seconds
}

/// Time-to-live for cache entries (seconds).
const CACHE_TTL_SECONDS: u64 = 120;

/// Session-scoped verification result cache.
/// Key: (tool_name, canonicalized_args_hash)
pub struct VerificationCache {
    inner: Mutex<HashMap<String, (VerificationCacheEntry, Instant)>>,
}

impl VerificationCache {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Compute deterministic cache key from tool name and arguments.
    pub fn make_key(tool_name: &str, args: &serde_json::Value) -> String {
        let mut hasher = Sha256::new();
        hasher.update(tool_name.as_bytes());
        if let Some(obj) = args.as_object() {
            let mut keys: Vec<&String> = obj.keys().collect();
            keys.sort();
            for k in keys {
                hasher.update(k.as_bytes());
                hasher.update(serde_json::to_vec(&obj[k]).unwrap_or_default());
            }
        } else {
            hasher.update(serde_json::to_vec(args).unwrap_or_default());
        }
        let result = hasher.finalize();
        format!("{}:{}", tool_name, hex::encode(result))
    }

    /// Get cached result if present and not expired.
    pub fn get(&self, tool_name: &str, args: &serde_json::Value) -> Option<VerificationCacheEntry> {
        let key = Self::make_key(tool_name, args);
        let map = self.inner.lock().unwrap();
        map.get(&key).and_then(|(entry, instant)| {
            if instant.elapsed() < Duration::from_secs(CACHE_TTL_SECONDS) {
                Some(entry.clone())
            } else {
                None
            }
        })
    }

    /// Store a verification result in the cache.
    pub fn set(&self, tool_name: &str, args: &serde_json::Value, safe: bool, reason: String) {
        let key = Self::make_key(tool_name, args);
        let entry = VerificationCacheEntry {
            safe,
            reason,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        };
        let mut map = self.inner.lock().unwrap();
        map.insert(key, (entry, Instant::now()));
    }

    /// Prune expired entries.
    pub fn prune(&self) {
        let mut map = self.inner.lock().unwrap();
        map.retain(|_, (_, instant)| instant.elapsed() < Duration::from_secs(CACHE_TTL_SECONDS));
    }
}

impl Default for VerificationCache {
    fn default() -> Self {
        Self::new()
    }
}
