use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub static CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("Failed to create reqwest client")
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
    let json = res.json::<T>().await?;
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
    let json = res.json::<T>().await?;
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
    let json: serde_json::Value = res.json().await.unwrap_or(serde_json::Value::Null);
    Ok((status, json))
}
