//! Models cache — fetching and caching available model lists from providers.

use crate::consts::models_cache_path;
use std::collections::HashMap;
use std::fs;

use super::ConfigManager;

impl ConfigManager {
    pub async fn get_cached_models(&self) -> HashMap<String, Vec<String>> {
        let c_path = models_cache_path();
        if !c_path.exists() {
            return self.update_models_cache().await;
        }

        let content = match fs::read_to_string(&c_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    path = %c_path.display(),
                    error = %e,
                    "Failed to read models cache; falling back to empty cache"
                );
                String::new()
            }
        };
        match serde_json::from_str(&content) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    path = %c_path.display(),
                    error = %e,
                    "Failed to parse models cache; falling back to empty cache"
                );
                HashMap::new()
            }
        }
    }

    pub fn get_cached_models_sync(&self) -> HashMap<String, Vec<String>> {
        let c_path = models_cache_path();
        if !c_path.exists() {
            return HashMap::new();
        }

        let content = match fs::read_to_string(&c_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    path = %c_path.display(),
                    error = %e,
                    "Failed to read models cache (sync); falling back to empty cache"
                );
                String::new()
            }
        };
        match serde_json::from_str(&content) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    path = %c_path.display(),
                    error = %e,
                    "Failed to parse models cache (sync); falling back to empty cache"
                );
                HashMap::new()
            }
        }
    }

    pub async fn update_models_cache(&self) -> HashMap<String, Vec<String>> {
        let providers = self.get_active_providers();
        let mut cache = HashMap::new();

        for p in providers {
            if let Ok(models) = self.fetch_models_from_provider(&p).await {
                cache.insert(p, models);
            }
        }

        if let Ok(content) = serde_json::to_string_pretty(&cache)
            && let Err(e) = fs::write(models_cache_path(), &content)
        {
            tracing::error!(
                path = %models_cache_path().display(),
                error = %e,
                "Failed to write models cache"
            );
        }
        cache
    }

    async fn fetch_models_from_provider(&self, provider: &str) -> anyhow::Result<Vec<String>> {
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
            req = req.header("Authorization", format!("Bearer {}", key));
        }

        let res = req.send().await?;
        let json: serde_json::Value = res.json().await?;

        let mut models = Vec::new();
        if provider == "ollama" && !url.contains("/v1") {
            if let Some(list) = json.get("models").and_then(|v| v.as_array()) {
                for m in list {
                    if let Some(name) = m.get("name").and_then(|v| v.as_str()) {
                        models.push(name.to_string());
                    }
                }
            }
        } else if let Some(data) = json.get("data").and_then(|v| v.as_array()) {
            for m in data {
                if let Some(id) = m.get("id").and_then(|v| v.as_str()) {
                    models.push(id.to_string());
                }
            }
        }

        Ok(models)
    }
}
