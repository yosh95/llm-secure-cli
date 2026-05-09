use crate::config::models::AppConfig;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

/// Update the current topic or strategic intent to keep the user informed.
pub fn update_topic(
    args: HashMap<String, Value>,
    _config: Arc<AppConfig>,
) -> anyhow::Result<Value> {
    let title = args
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Progress Update");
    let summary = args.get("summary").and_then(|v| v.as_str());
    let strategic_intent = args.get("strategic_intent").and_then(|v| v.as_str());

    crate::cli::ui::print_topic(title, summary, strategic_intent);

    Ok(json!({
        "status": "success",
        "message": "Topic updated and displayed to the user."
    }))
}
