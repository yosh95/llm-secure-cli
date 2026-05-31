//! Models cache — fetching and caching available model lists from providers.

use crate::consts::models_cache_path;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;

use super::ConfigManager;

/// A single model entry in the cache.
///
/// Supports two formats for backward compatibility:
/// - Old format: a plain string (model ID) — `supports_tools` defaults to `true`.
/// - New format: an object with `id` and `supports_tools`.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum CachedModelEntry {
    /// Old format: just a model ID string.  `supports_tools` is assumed `true`.
    Simple(String),
    /// New format: full model entry with metadata.
    Detailed {
        id: String,
        #[serde(default = "return_true")]
        supports_tools: bool,
    },
}

fn return_true() -> bool {
    true
}

impl CachedModelEntry {
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            CachedModelEntry::Simple(id) => id,
            CachedModelEntry::Detailed { id, .. } => id,
        }
    }

    /// Whether this model supports tool (function) calling.
    ///
    /// For entries in the old format (plain strings) the value defaults to `true`
    /// so that existing cache files remain valid.
    #[must_use]
    pub fn supports_tools(&self) -> bool {
        match self {
            CachedModelEntry::Simple(_) => true,
            CachedModelEntry::Detailed { supports_tools, .. } => *supports_tools,
        }
    }
}

impl From<String> for CachedModelEntry {
    fn from(id: String) -> Self {
        CachedModelEntry::Simple(id)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers for reading / writing the on-disk cache
// ---------------------------------------------------------------------------

/// Read the cache file and deserialise it.
///
/// Returns `Ok(Some(map))` if the file exists and parses correctly,
/// `Ok(None)` if the file does not exist, or `Err` on I/O / parse errors.
fn read_cache() -> anyhow::Result<Option<HashMap<String, Vec<CachedModelEntry>>>> {
    let c_path = models_cache_path();
    if !c_path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&c_path)?;
    match serde_json::from_str(&content) {
        Ok(map) => Ok(Some(map)),
        Err(e) => {
            // Try the old format (HashMap<String, Vec<String>>) for backward compat.
            if let Ok(old_map) = serde_json::from_str::<HashMap<String, Vec<String>>>(&content) {
                let converted: HashMap<String, Vec<CachedModelEntry>> = old_map
                    .into_iter()
                    .map(|(k, v)| (k, v.into_iter().map(CachedModelEntry::Simple).collect()))
                    .collect();
                return Ok(Some(converted));
            }
            Err(anyhow::anyhow!("Failed to parse models cache: {e}"))
        }
    }
}

/// Write the cache to disk.
fn write_cache(cache: &HashMap<String, Vec<CachedModelEntry>>) {
    if let Ok(content) = serde_json::to_string_pretty(cache)
        && let Err(e) = fs::write(models_cache_path(), &content)
    {
        tracing::error!(
            path = %models_cache_path().display(),
            error = %e,
            "Failed to write models cache"
        );
    }
}

// ---------------------------------------------------------------------------
// Public API on ConfigManager
// ---------------------------------------------------------------------------

impl ConfigManager {
    /// Return the cached model IDs for each provider.
    /// If the cache does not exist it is fetched from providers first.
    pub async fn get_cached_models(&self) -> HashMap<String, Vec<String>> {
        let map = match read_cache() {
            Ok(Some(m)) => m,
            Ok(None) => return self.update_models_cache().await,
            Err(e) => {
                tracing::warn!(error = %e, "Falling back to fetching models");
                return self.update_models_cache().await;
            }
        };
        map.into_iter()
            .map(|(k, v)| (k, v.into_iter().map(|e| e.id().to_string()).collect()))
            .collect()
    }

    /// Synchronous variant — does **not** trigger a fetch if the cache is
    /// missing; returns an empty map instead.
    pub fn get_cached_models_sync(&self) -> HashMap<String, Vec<String>> {
        let map = match read_cache() {
            Ok(Some(m)) => m,
            _ => return HashMap::new(),
        };
        map.into_iter()
            .map(|(k, v)| (k, v.into_iter().map(|e| e.id().to_string()).collect()))
            .collect()
    }

    /// Return `Some(true/false)` if the cache has metadata about whether a given
    /// model supports tool calling.  Returns `None` when the model is not in the
    /// cache (callers should default to `true` for backward compatibility).
    pub fn model_supports_tools(&self, provider: &str, model: &str) -> Option<bool> {
        let map = match read_cache() {
            Ok(Some(m)) => m,
            _ => return None,
        };
        map.get(provider)
            .and_then(|models| models.iter().find(|e| e.id() == model))
            .map(CachedModelEntry::supports_tools)
    }

    /// Fetch model lists from all active providers and persist the combined
    /// cache.  Returns the model IDs (string form) for convenience.
    pub async fn update_models_cache(&self) -> HashMap<String, Vec<String>> {
        let providers = self.get_active_providers();
        let mut cache: HashMap<String, Vec<CachedModelEntry>> = HashMap::new();

        for p in providers {
            if let Ok(models) = self.fetch_models_from_provider(&p).await {
                cache.insert(p, models);
            }
        }

        write_cache(&cache);

        // Return the old-style map for callers that expect it.
        cache
            .into_iter()
            .map(|(k, v)| (k, v.into_iter().map(|e| e.id().to_string()).collect()))
            .collect()
    }

    /// Fetch model data from a single provider.
    ///
    /// For providers whose API exposes per-model capabilities (e.g. `OpenRouter`
    /// with its `supported_parameters` array), the returned entries carry
    /// metadata that can be used to decide whether to send tool definitions.
    async fn fetch_models_from_provider(
        &self,
        provider: &str,
    ) -> anyhow::Result<Vec<CachedModelEntry>> {
        let api_key = self.get_api_key(provider);
        let config = self.get_config()?;

        let mut url = String::new();
        if let Some(p_cfg) = config.providers.get(provider)
            && let Some(api_url) = &p_cfg.api_url
        {
            url = api_url.clone();
        }

        if url.is_empty() {
            match provider {
                "openai" => url = "https://api.openai.com/v1".to_string(),
                "openrouter" => url = "https://openrouter.ai/api/v1".to_string(),
                "ollama" => url = "http://localhost:11434/v1".to_string(),
                _ => return Err(anyhow::anyhow!("No API URL for provider")),
            }
        }

        let models_url = if provider == "ollama" && !url.contains("/v1") {
            format!("{}/api/tags", url.trim_end_matches('/'))
        } else if provider == "openrouter" && url == "https://openrouter.ai/api/v1" {
            // openrouter requires query param to include all modalities like video/image generation
            "https://openrouter.ai/api/v1/models?output_modalities=all".to_string()
        } else {
            format!("{}/models", url.trim_end_matches('/'))
        };

        let mut req = crate::utils::http::CLIENT.get(&models_url);
        if let Some(key) = api_key
            && key != "local_bypass"
        {
            req = req.header("Authorization", format!("Bearer {key}"));
        }

        let res = req.send().await?;
        let json: serde_json::Value = res.json().await?;

        let mut models = Vec::new();
        if provider == "ollama" && !url.contains("/v1") {
            // Ollama's /api/tags returns {"models": [{"name": "...", ...}]}
            if let Some(list) = json.get("models").and_then(|v| v.as_array()) {
                for m in list {
                    if let Some(name) = m.get("name").and_then(|v| v.as_str()) {
                        // Ollama models generally support tools
                        models.push(CachedModelEntry::Detailed {
                            id: name.to_string(),
                            supports_tools: true,
                        });
                    }
                }
            }
        } else if let Some(data) = json.get("data").and_then(|v| v.as_array()) {
            // OpenAI-compatible /models endpoint — includes OpenRouter.
            for m in data {
                if let Some(id) = m.get("id").and_then(|v| v.as_str()) {
                    // Check for tool-support metadata (OpenRouter /models API).
                    let supports_tools = m
                        .get("supported_parameters")
                        .and_then(|v| v.as_array())
                        .is_none_or(|params| params.iter().any(|p| p.as_str() == Some("tools")));
                    models.push(CachedModelEntry::Detailed {
                        id: id.to_string(),
                        supports_tools,
                    });
                }
            }
        }

        Ok(models)
    }
}
