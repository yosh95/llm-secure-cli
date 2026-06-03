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
        /// Model modality type: "chat" (text generation/chat), "image" (image generation),
        /// "video" (video generation), "audio" (audio generation/speech).
        /// When None/unset, defaults to "chat" for backward compatibility.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model_type: Option<String>,
        /// Input modalities this model supports (e.g. ["text", "image", "audio", "file"]).
        /// Populated from OpenRouter's `architecture.input_modalities` field.
        /// When None/unset, all modalities are assumed supported for backward compatibility.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input_modalities: Option<Vec<String>>,
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

    /// Returns the model modality type: "chat", "image", "video", or "audio".
    /// Defaults to "chat" when not set (backward compatibility).
    #[must_use]
    pub fn model_type(&self) -> &str {
        match self {
            CachedModelEntry::Simple(_) => "chat",
            CachedModelEntry::Detailed { model_type, .. } => {
                model_type.as_deref().unwrap_or("chat")
            }
        }
    }

    /// Returns the input modalities this model supports (e.g. ["text", "image"]).
    /// Returns `None` when unknown (defaults to assuming all inputs are supported
    /// for backward compatibility).
    #[must_use]
    pub fn input_modalities(&self) -> Option<&[String]> {
        match self {
            CachedModelEntry::Simple(_) => None,
            CachedModelEntry::Detailed {
                input_modalities, ..
            } => input_modalities.as_deref(),
        }
    }

    /// Returns `true` if this model supports the given input modality (e.g. "image", "audio").
    /// When modality info is unavailable (None), conservatively returns `true` for
    /// backward compatibility.
    #[must_use]
    pub fn supports_input_modality(&self, modality: &str) -> bool {
        match self {
            CachedModelEntry::Simple(_) => true,
            CachedModelEntry::Detailed {
                input_modalities, ..
            } => input_modalities
                .as_ref()
                .is_none_or(|mods| mods.iter().any(|m| m == modality)),
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

    /// Return the model type for a given provider/model from the cache.
    ///
    /// Returns `Some(Some(type_str))` if the model is found with a known type,
    /// `Some(None)` if found but no type set (defaults to "chat"),
    /// or `None` if the model is not in the cache at all.
    pub fn model_type(&self, provider: &str, model: &str) -> Option<Option<String>> {
        let map = match read_cache() {
            Ok(Some(m)) => m,
            _ => return None,
        };
        map.get(provider)
            .and_then(|models| models.iter().find(|e| e.id() == model))
            .map(|e| match e {
                CachedModelEntry::Simple(_) => None,
                CachedModelEntry::Detailed { model_type, .. } => model_type.clone(),
            })
    }

    /// Return the input modalities for a given provider/model from the cache.
    ///
    /// Returns `Some(Some(vec))` if the model is found with known input modalities,
    /// `Some(None)` if found but no modality info (assume all inputs supported),
    /// or `None` if the model is not in the cache at all.
    pub fn model_input_modalities(
        &self,
        provider: &str,
        model: &str,
    ) -> Option<Option<Vec<String>>> {
        let map = match read_cache() {
            Ok(Some(m)) => m,
            _ => return None,
        };
        map.get(provider)
            .and_then(|models| models.iter().find(|e| e.id() == model))
            .map(|e| match e {
                CachedModelEntry::Simple(_) => None,
                CachedModelEntry::Detailed {
                    input_modalities, ..
                } => input_modalities.clone(),
            })
    }

    /// Check if a given provider/model supports a specific input modality (e.g. "image", "audio").
    /// Returns `None` when the model is not in the cache (callers should assume supported).
    /// Returns `Some(true/false)` based on cached modality info.
    pub fn model_supports_input_modality(
        &self,
        provider: &str,
        model: &str,
        modality: &str,
    ) -> Option<bool> {
        let map = match read_cache() {
            Ok(Some(m)) => m,
            _ => return None,
        };
        map.get(provider)
            .and_then(|models| models.iter().find(|e| e.id() == model))
            .map(|e| e.supports_input_modality(modality))
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
                            model_type: None,
                            input_modalities: None,
                        });
                    }
                }
            }
            if let Some(data) = json.get("data").and_then(|v| v.as_array()) {
                for m in data {
                    if let Some(id) = m.get("id").and_then(|v| v.as_str()) {
                        let tags: Vec<&str> = m
                            .get("metadata")
                            .and_then(|v| v.get("tags"))
                            .and_then(|v| v.as_array())
                            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                            .unwrap_or_default();

                        // Skip embedding models — irrelevant for chat/image/audio inference
                        if tags.contains(&"embed") {
                            continue;
                        }

                        let supports_tools = tags.contains(&"chat");

                        // Determine model type from metadata tags
                        let model_type = if tags.contains(&"image-gen") {
                            Some("image".to_string())
                        } else if tags.contains(&"stt") || tags.contains(&"tts") {
                            Some("audio".to_string())
                        } else {
                            Some("chat".to_string())
                        };

                        models.push(CachedModelEntry::Detailed {
                            id: id.to_string(),
                            supports_tools,
                            model_type,
                            input_modalities: None,
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
                    // Parse input_modalities from architecture field (OpenRouter API).
                    let input_modalities = m
                        .get("architecture")
                        .and_then(|a| a.get("input_modalities"))
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect::<Vec<String>>()
                        });

                    models.push(CachedModelEntry::Detailed {
                        id: id.to_string(),
                        supports_tools,
                        model_type: None,
                        input_modalities,
                    });
                }
            }
        }

        Ok(models)
    }
}
