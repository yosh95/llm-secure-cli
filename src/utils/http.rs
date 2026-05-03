use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub static CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .gzip(true)
        .build()
        .expect("Failed to create reqwest client")
});

pub async fn get_json<T: for<'de> Deserialize<'de> + Send + 'static>(
    url: String,
    headers: HashMap<String, String>,
) -> anyhow::Result<T> {
    tracing::debug!("HTTP GET Request: URL: {}", url);
    let mut req = CLIENT.get(&url);
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let res = req.send().await?;
    let status = res.status();
    let text = res.text().await?;
    tracing::debug!("HTTP GET Response: Status: {}, Body: {}", status, text);
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
    tracing::debug!(
        "HTTP POST Request: URL: {}, Body: {}",
        url,
        serde_json::to_string(&body).unwrap_or_default()
    );
    let mut req = CLIENT.post(&url);
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let res = req.json(&body).send().await?;
    let status = res.status();
    let text = res.text().await?;
    tracing::debug!("HTTP POST Response: Status: {}, Body: {}", status, text);
    let json = serde_json::from_str::<T>(&text)?;
    Ok(json)
}

pub async fn post_json_with_status<B: Serialize + Send + 'static>(
    url: String,
    headers: HashMap<String, String>,
    body: B,
) -> anyhow::Result<(u16, serde_json::Value)> {
    tracing::debug!(
        "HTTP POST Request (with status): URL: {}, Body: {}",
        url,
        serde_json::to_string(&body).unwrap_or_default()
    );
    let mut req_builder = CLIENT.post(&url);
    for (k, v) in headers {
        req_builder = req_builder.header(k, v);
    }

    let res = req_builder.json(&body).send().await?;

    let status = res.status().as_u16();
    let text = res.text().await.unwrap_or_default();
    tracing::debug!(
        "HTTP POST Response (with status): Status: {}, Body: {}",
        status,
        text
    );
    let json: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
    Ok((status, json))
}
