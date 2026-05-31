use chrono::Local;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Model,
    Tool,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct ContentPart {
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inline_data: Option<HashMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_call: Option<HashMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_response: Option<HashMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<HashMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_response: Option<HashMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thought: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thought_signature: Option<String>,
    #[serde(default)]
    pub is_diagnostic: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum MessagePart {
    Text(String),
    Part(Box<ContentPart>),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Message {
    pub role: Role,
    pub parts: Vec<MessagePart>,
}

impl Message {
    #[must_use]
    pub fn get_text(&self, include_diagnostic: bool) -> String {
        let mut text_parts = Vec::new();
        for p in &self.parts {
            match p {
                MessagePart::Text(t) => text_parts.push(t.clone()),
                MessagePart::Part(cp) => {
                    if let Some(t) = &cp.text
                        && (!cp.is_diagnostic || include_diagnostic)
                    {
                        text_parts.push(t.clone());
                    }
                }
            }
        }
        text_parts.join("")
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DataSource {
    pub content: serde_json::Value,
    pub content_type: String,
    pub is_file_or_url: bool,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, serde_json::Value>,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Default)]
pub struct LlmResponse {
    pub content: Option<String>,
    pub tool_name: Option<String>,
    pub usage: Option<Usage>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ClientState {
    pub model: String,
    pub provider: String,
    pub conversation: Vec<Message>,
    pub tools_enabled: bool,
    pub system_prompt_enabled: bool,
    pub system_prompt: Option<String>,
    pub stdout: bool,
    pub render_markdown: bool,
}

impl ClientState {
    #[must_use]
    pub fn get_effective_system_prompt(&self) -> Option<String> {
        if !self.system_prompt_enabled {
            return None;
        }

        let date_str = Local::now().format("%Y-%m-%d").to_string();
        let directive = format!(
            "Today's date is {date_str}. You must treat this as the current date and ignore your training cutoff or any other date information."
        );

        match &self.system_prompt {
            Some(sp) if !sp.is_empty() => Some(format!("{sp}\n\n{directive}")),
            _ => Some(directive),
        }
    }
}
