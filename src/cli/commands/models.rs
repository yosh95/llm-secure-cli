use crate::cli::ui;
use crate::config::CONFIG_MANAGER;
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::Value;

pub async fn list_models(provider: &str, models: Vec<String>, verbose: bool) {
    let api_key = CONFIG_MANAGER.get_api_key(provider);
    if api_key.is_none() && provider != "ollama" {
        ui::report_error(&format!("{} API Key not found.", provider));
        return;
    }

    let client = reqwest::Client::new();
    let mut headers = HeaderMap::new();

    let url = match provider {
        "openai" | "gpt" => {
            if let Some(key) = api_key {
                headers.insert(
                    "Authorization",
                    HeaderValue::from_str(&format!("Bearer {}", key)).unwrap(),
                );
            }
            "https://api.openai.com/v1/models".to_string()
        }
        "anthropic" | "claude" => {
            if let Some(key) = api_key {
                headers.insert("x-api-key", HeaderValue::from_str(&key).unwrap());
            }
            headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
            "https://api.anthropic.com/v1/models".to_string()
        }
        "google" | "gemini" => {
            if let Some(key) = api_key {
                headers.insert("x-goog-api-key", HeaderValue::from_str(&key).unwrap());
            }
            "https://generativelanguage.googleapis.com/v1beta/models".to_string()
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
            format!("{}/api/tags", base_url)
        }
        _ => {
            ui::report_error(&format!("Unknown provider: {}", provider));
            return;
        }
    };

    match client.get(&url).headers(headers).send().await {
        Ok(response) => {
            if !response.status().is_success() {
                ui::report_error(&format!("Failed to fetch models: {}", response.status()));
                return;
            }
            let result: Value = response.json().await.unwrap_or(Value::Null);
            let models_data = match provider {
                "openai" | "gpt" | "anthropic" | "claude" => result.get("data"),
                "google" | "gemini" => result.get("models"),
                "ollama" => result.get("models"),
                _ => None,
            };

            if let Some(models_list) = models_data.and_then(|m| m.as_array()) {
                let mut sorted_models = models_list.clone();
                sorted_models.sort_by(|a, b| {
                    let get_id = |m: &Value| {
                        match provider {
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
                        let id = match provider {
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
                        match provider {
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
                        let id = match provider {
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
        Err(e) => {
            ui::report_error(&format!("Error fetching models: {}", e));
        }
    }
}
