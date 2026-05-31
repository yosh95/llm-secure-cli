use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::LazyLock;

pub static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    let version = env!("CARGO_PKG_VERSION");
    let ua = format!("llm-secure-cli/{version} (https://github.com/yosh95/llm-secure-cli)");
    reqwest::Client::builder()
        .user_agent(ua)
        .timeout(std::time::Duration::from_secs(30))
        .gzip(true)
        .build()
        .unwrap_or_else(|e| {
            tracing::error!(
                error = %e,
                "CRITICAL: Failed to create global reqwest client; using fallback with no timeout"
            );
            reqwest::Client::new()
        })
});

pub async fn get_json<T: for<'de> Deserialize<'de> + Send + 'static>(
    url: String,
    headers: HashMap<String, String>,
) -> anyhow::Result<T> {
    let mut req = CLIENT.get(&url);
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let res = req.send().await?;
    let _status = res.status();
    let text = res.text().await?;
    let json = serde_json::from_str::<T>(&text)?;
    Ok(json)
}

pub async fn post_json<
    T: for<'de> Deserialize<'de> + Send + 'static,
    B: Serialize + Send + 'static,
>(
    url: String,
    headers: HashMap<String, String>,
    body: B,
) -> anyhow::Result<T> {
    let mut req = CLIENT.post(&url);
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let res = req.json(&body).send().await?;
    let _status = res.status();
    let text = res.text().await?;
    let json = serde_json::from_str::<T>(&text)?;
    Ok(json)
}

pub async fn post_json_with_status<B: Serialize + Send + 'static>(
    url: String,
    headers: HashMap<String, String>,
    body: B,
) -> anyhow::Result<(u16, serde_json::Value)> {
    let mut req_builder = CLIENT.post(&url);
    for (k, v) in headers {
        req_builder = req_builder.header(k, v);
    }

    let res = req_builder.json(&body).send().await?;

    let status = res.status().as_u16();
    let text = match res.text().await {
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
}
