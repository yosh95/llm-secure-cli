use serde_json::{Value, json};

/// Trait to handle provider-specific payload formatting.
/// This decouples the generic client from specific API quirks (`OpenRouter`, Anthropic, etc.)
pub trait PayloadFormatter: Send + Sync {
    fn format_text(&self, text: &str) -> Value {
        json!({"type": "text", "text": text})
    }
    fn format_image(&self, mime: &str, data: &str) -> Value {
        json!({
            "type": "image_url",
            "image_url": { "url": format!("data:{};base64,{}", mime, data) }
        })
    }
    fn format_pdf(&self, _data: &str, _filename: Option<&str>) -> Option<Value>;
    fn format_audio(&self, mime: &str, data: &str) -> Value {
        json!({
            "type": "input_audio",
            "input_audio": {
                "data": data,
                "format": mime.split('/').next_back().unwrap_or("mp3")
            }
        })
    }
}

pub struct GenericPayloadFormatter;
impl PayloadFormatter for GenericPayloadFormatter {
    fn format_pdf(&self, data: &str, _filename: Option<&str>) -> Option<Value> {
        // Default OpenAI compatibility: treat as image or ignore if not supported
        Some(json!({
            "type": "image_url",
            "image_url": { "url": format!("data:application/pdf;base64,{}", data) }
        }))
    }
}

pub struct HighFeaturePayloadFormatter {
    pub is_anthropic_gemini: bool,
}
impl PayloadFormatter for HighFeaturePayloadFormatter {
    fn format_pdf(&self, data: &str, _filename: Option<&str>) -> Option<Value> {
        if self.is_anthropic_gemini {
            // Anthropic/Gemini style native PDF support
            Some(json!({
                "type": "document",
                "source": { "type": "base64", "media_type": "application/pdf", "data": data }
            }))
        } else {
            // Default to image_url fallback
            Some(json!({
                "type": "image_url",
                "image_url": { "url": format!("data:application/pdf;base64,{}", data) }
            }))
        }
    }
}
