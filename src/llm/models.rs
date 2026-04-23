use chrono::Local;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Model,
    Tool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ContentPart {
    pub text: Option<String>,
    pub inline_data: Option<HashMap<String, serde_json::Value>>,
    pub function_call: Option<HashMap<String, serde_json::Value>>,
    pub function_response: Option<HashMap<String, serde_json::Value>>,
    pub thought: Option<String>,
    pub thought_signature: Option<String>,
    #[serde(default)]
    pub is_diagnostic: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum MessagePart {
    Text(String),
    Part(ContentPart),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Message {
    pub role: Role,
    pub parts: Vec<MessagePart>,
}

impl Message {
    pub fn get_text(&self, include_diagnostic: bool) -> String {
        let mut text_parts = Vec::new();
        for p in &self.parts {
            match p {
                MessagePart::Text(t) => text_parts.push(t.clone()),
                MessagePart::Part(cp) => {
                    if let Some(t) = &cp.text
                        && (!cp.is_diagnostic || include_diagnostic) {
                            text_parts.push(t.clone());
                        }
                }
            }
        }
        text_parts.join("")
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DataSource {
    pub content: serde_json::Value,
    pub content_type: String,
    pub is_file_or_url: bool,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ClientState {
    pub model: String,
    pub provider: String,
    pub conversation: Vec<Message>,
    pub tools_enabled: bool,
    pub system_prompt_enabled: bool,
    pub system_prompt: Option<String>,
    pub stdout: bool,
    pub render_markdown: bool,
    pub live_debug: bool,
}

impl ClientState {
    pub fn get_effective_system_prompt(&self) -> Option<String> {
        if !self.system_prompt_enabled {
            return None;
        }

        let date_str = Local::now().format("%Y-%m-%d").to_string();
        let directive = format!(
            "Today's date is {}. You must treat this as the current date and ignore your training cutoff or any other date information.",
            date_str
        );

        match &self.system_prompt {
            Some(sp) if !sp.is_empty() => Some(format!("{}\n\n{}", directive, sp)),
            _ => Some(directive),
        }
    }
}
