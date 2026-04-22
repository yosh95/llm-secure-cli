use crate::cli::ui;
use crate::config::CONFIG_MANAGER;
use serde_json::Value;

pub async fn list_models(provider_name: &str, models: Vec<String>, verbose: bool) {
    let provider = provider_name.to_lowercase();
    let api_key = CONFIG_MANAGER.get_api_key(&provider);
    if api_key.is_none() && provider != "ollama" {
        ui::report_error(&format!("{} API Key not found.", provider));
        return;
    }

    let provider_clone = provider.clone();
    let api_key_clone = api_key.clone();

    let fetch_result = tokio::task::spawn_blocking(move || {
        let (url, headers) = match provider_clone.as_str() {
            "openai" | "gpt" => {
                let mut h = Vec::new();
                if let Some(key) = &api_key_clone {
                    h.push(("Authorization".to_string(), format!("Bearer {}", key)));
                }
                ("https://api.openai.com/v1/models".to_string(), h)
            }
            "anthropic" | "claude" => {
                let mut h = Vec::new();
                if let Some(key) = &api_key_clone {
                    h.push(("x-api-key".to_string(), key.clone()));
                }
                h.push(("anthropic-version".to_string(), "2023-06-01".to_string()));
                ("https://api.anthropic.com/v1/models".to_string(), h)
            }
            "google" | "gemini" => {
                let mut h = Vec::new();
                if let Some(key) = &api_key_clone {
                    h.push(("x-goog-api-key".to_string(), key.clone()));
                }
                (
                    "https://generativelanguage.googleapis.com/v1beta/models".to_string(),
                    h,
                )
            }
            "ollama" => {
                let config = CONFIG_MANAGER.get_config();
                let mut base_url = "http://localhost:11434".to_string();
                if let Some(p_cfg) = config.providers.get("ollama") {
                    if let Some(api_url) = &p_cfg.api_url {
                        if api_url.contains("/v1") {
                            base_url = api_url.split("/v1").next().unwrap().to_string();
                        } else {
                            base_url = api_url.clone();
                        }
                    }
                }
                (format!("{}/api/tags", base_url), Vec::new())
            }
            _ => {
                return Err(anyhow::anyhow!("Unknown provider: {}", provider_clone));
            }
        };

        let mut req = ureq::get(&url);
        for (k, v) in headers {
            req = req.header(&k, &v);
        }
        let res = req.call()?;
        let json: Value = res.into_body().read_json()?;
        Ok(json)
    })
    .await;

    match fetch_result {
        Ok(Ok(result)) => {
            let provider_str = provider.as_str();
            let models_data = match provider_str {
                "openai" | "gpt" | "anthropic" | "claude" => result.get("data"),
                "google" | "gemini" => result.get("models"),
                "ollama" => result.get("models"),
                _ => None,
            };

            if let Some(models_list) = models_data.and_then(|m| m.as_array()) {
                let mut sorted_models = models_list.clone();
                sorted_models.sort_by(|a, b| {
                    let get_id = |m: &Value| {
                        match provider.as_str() {
                            "openai" | "gpt" | "anthropic" | "claude" => {
                                m.get("id").and_then(|v| v.as_str()).map(|s| s.to_string())
                            }
                            "google" | "gemini" => m.get("name").and_then(|v| {
                                v.as_str()
                                    .and_then(|s| s.split('/').next_back())
                                    .map(|s| s.to_string())
                            }),
                            "ollama" => m
                                .get("name")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            _ => None,
                        }
                        .unwrap_or_default()
                    };
                    get_id(a).cmp(&get_id(b))
                });

                if !models.is_empty() {
                    for m in &sorted_models {
                        let id = match provider.as_str() {
                            "openai" | "gpt" | "anthropic" | "claude" => {
                                m.get("id").and_then(|v| v.as_str())
                            }
                            "google" | "gemini" => m
                                .get("name")
                                .and_then(|v| v.as_str().and_then(|s| s.split('/').next_back())),
                            "ollama" => m.get("name").and_then(|v| v.as_str()),
                            _ => None,
                        };
                        if let Some(id_str) = id {
                            if models.contains(&id_str.to_string()) {
                                println!("{}", serde_json::to_string_pretty(m).unwrap());
                            }
                        }
                    }
                } else if verbose {
                    println!("{:<30} {:<30}", "Model ID", "Details");
                    println!("{:-<60}", "");
                    for m in &sorted_models {
                        match provider.as_str() {
                            "openai" | "gpt" => {
                                println!(
                                    "{:<30} {:<20}",
                                    m.get("id").and_then(|v| v.as_str()).unwrap_or("N/A"),
                                    m.get("owned_by").and_then(|v| v.as_str()).unwrap_or("N/A")
                                );
                            }
                            "anthropic" | "claude" => {
                                println!(
                                    "{:<30} {:<30}",
                                    m.get("id").and_then(|v| v.as_str()).unwrap_or("N/A"),
                                    m.get("display_name")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("N/A")
                                );
                            }
                            "google" | "gemini" => {
                                println!(
                                    "{:<30} {:<30}",
                                    m.get("name")
                                        .and_then(|v| v
                                            .as_str()
                                            .and_then(|s| s.split('/').next_back()))
                                        .unwrap_or("N/A"),
                                    m.get("displayName")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("N/A")
                                );
                            }
                            "ollama" => {
                                println!(
                                    "{:<30} {:<10}",
                                    m.get("name").and_then(|v| v.as_str()).unwrap_or("N/A"),
                                    format!(
                                        "{:.2} GB",
                                        m.get("size").and_then(|v| v.as_f64()).unwrap_or(0.0)
                                            / (1024.0 * 1024.0 * 1024.0)
                                    )
                                );
                            }
                            _ => {}
                        }
                    }
                } else {
                    for m in &sorted_models {
                        let id = match provider.as_str() {
                            "openai" | "gpt" | "anthropic" | "claude" => {
                                m.get("id").and_then(|v| v.as_str())
                            }
                            "google" | "gemini" => m
                                .get("name")
                                .and_then(|v| v.as_str().and_then(|s| s.split('/').next_back())),
                            "ollama" => m.get("name").and_then(|v| v.as_str()),
                            _ => None,
                        };
                        if let Some(id_str) = id {
                            println!("{}", id_str);
                        }
                    }
                }
            } else {
                println!("{}", serde_json::to_string_pretty(&result).unwrap());
            }
        }
        Ok(Err(e)) => {
            ui::report_error(&format!("Error fetching models: {}", e));
        }
        Err(e) => {
            ui::report_error(&format!("Task join error: {}", e));
        }
    }
}
