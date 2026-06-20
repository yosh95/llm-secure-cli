use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::LazyLock;

pub static CLIENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
    let version = env!("CARGO_PKG_VERSION");
    let ua = format!("llm-secure-cli/{version} (https://github.com/yosh95/llm-secure-cli)");
    let config = ureq::config::Config::builder()
        .user_agent(ua)
        .timeout_connect(Some(std::time::Duration::from_secs(10)))
        .timeout_recv_body(Some(std::time::Duration::from_secs(30)))
        .timeout_send_body(Some(std::time::Duration::from_secs(30)))
        .build();
    ureq::Agent::new_with_config(config)
});

pub fn get_json<T: for<'de> Deserialize<'de> + Send + 'static>(
    url: String,
    headers: HashMap<String, String>,
) -> anyhow::Result<T> {
    crate::core::session::run_cancellable(move || {
        let mut req = CLIENT.get(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let response = req
            .call()
            .map_err(|e| anyhow::anyhow!("HTTP GET request failed: {e}"))?;
        let text = response
            .into_body()
            .read_to_string()
            .map_err(|e| anyhow::anyhow!("Failed to read response body: {e}"))?;
        let json = serde_json::from_str::<T>(&text)
            .map_err(|e| anyhow::anyhow!("Failed to parse JSON response: {e}"))?;
        Ok(json)
    })
}

pub fn post_json<T: for<'de> Deserialize<'de> + Send + 'static, B: Serialize + Send + 'static>(
    url: String,
    headers: HashMap<String, String>,
    body: B,
) -> anyhow::Result<T> {
    crate::core::session::run_cancellable(move || {
        let mut req = CLIENT.post(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let response = req
            .send_json(&body)
            .map_err(|e| anyhow::anyhow!("HTTP POST request failed: {e}"))?;
        let text = response
            .into_body()
            .read_to_string()
            .map_err(|e| anyhow::anyhow!("Failed to read response body: {e}"))?;
        let json = serde_json::from_str::<T>(&text)
            .map_err(|e| anyhow::anyhow!("Failed to parse JSON response: {e}"))?;
        Ok(json)
    })
}

pub fn post_json_with_status<B: Serialize + Send + 'static>(
    url: String,
    headers: HashMap<String, String>,
    body: B,
) -> anyhow::Result<(u16, serde_json::Value)> {
    crate::core::session::run_cancellable(move || {
        let mut req = CLIENT.post(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let response = match req.send_json(&body) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "HTTP POST request failed");
                return Ok((0u16, serde_json::Value::Null));
            }
        };
        let status = response.status().as_u16();
        let text = match response.into_body().read_to_string() {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to read response body for status {}", status);
                String::new()
            }
        };
        let json: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    body_preview = %&text[..text.len().min(200)],
                    "Failed to parse JSON response body"
                );
                serde_json::Value::Null
            }
        };
        Ok((status, json))
    })
}
