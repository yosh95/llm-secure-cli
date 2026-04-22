use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub async fn get_json<T: for<'de> Deserialize<'de> + Send + 'static>(
    url: String,
    headers: HashMap<String, String>,
) -> anyhow::Result<T> {
    tokio::task::spawn_blocking(move || {
        let mut req = ureq::get(&url);
        for (k, v) in headers {
            req = req.header(k, v);
        }
        let res = req.call()?;
        let json = res.into_body().read_json::<T>()?;
        Ok(json)
    })
    .await?
}

pub async fn post_json<
    T: for<'de> Deserialize<'de> + Send + 'static,
    B: Serialize + Send + 'static,
>(
    url: String,
    headers: HashMap<String, String>,
    body: B,
) -> anyhow::Result<T> {
    tokio::task::spawn_blocking(move || {
        let mut req = ureq::post(&url);
        for (k, v) in headers {
            req = req.header(k, v);
        }
        let res = req.send_json(body)?;
        let json = res.into_body().read_json::<T>()?;
        Ok(json)
    })
    .await?
}

pub async fn post_json_with_status<B: Serialize + Send + 'static>(
    url: String,
    headers: HashMap<String, String>,
    body: B,
) -> anyhow::Result<(u16, serde_json::Value)> {
    tokio::task::spawn_blocking(move || {
        let mut req_builder = ureq::post(&url);
        for (k, v) in headers {
            req_builder = req_builder.header(k, v);
        }

        let res = req_builder
            .config()
            .http_status_as_error(false)
            .build()
            .send_json(body)?;

        let status = res.status().as_u16();
        let json: serde_json::Value = res
            .into_body()
            .read_json()
            .unwrap_or(serde_json::Value::Null);
        Ok((status, json))
    })
    .await?
}
