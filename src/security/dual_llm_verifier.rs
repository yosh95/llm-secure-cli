use crate::config::CONFIG_MANAGER;
use crate::llm::base::LlmClient;
use crate::llm::models::DataSource;
use crate::llm::registry::CLIENT_REGISTRY;
use serde_json::json;
use std::collections::HashMap;

pub struct DualLlmVerifier {
    pub client: Box<dyn LlmClient>,
}

impl DualLlmVerifier {
    pub fn new(client: Box<dyn LlmClient>) -> Self {
        Self { client }
    }

    pub async fn verify(
        &mut self,
        intent: &str,
        tool_name: &str,
        tool_args: &serde_json::Value,
    ) -> anyhow::Result<bool> {
        let (safe, _) = verify_tool_call(intent, tool_name, tool_args, None).await;
        Ok(safe)
    }
}

pub async fn verify_tool_call(
    user_prompt: &str,
    tool_name: &str,
    args: &serde_json::Value,
    last_tool_result: Option<&str>,
) -> (bool, String) {
    verify_tool_call_full(user_prompt, tool_name, args, last_tool_result, None, None).await
}

pub async fn verify_tool_call_full(
    user_prompt: &str,
    tool_name: &str,
    args: &serde_json::Value,
    last_tool_result: Option<&str>,
    provider_override: Option<String>,
    model_override: Option<String>,
) -> (bool, String) {
    let config = CONFIG_MANAGER.get_config();
    let provider_alias =
        provider_override.unwrap_or_else(|| config.security.dual_llm_provider.clone());
    let model_alias = model_override.unwrap_or_else(|| config.security.dual_llm_model.clone());

    let client = {
        let registry = CLIENT_REGISTRY.lock().unwrap();
        registry.create_client(&provider_alias, &model_alias, false, true)
    };

    let mut client = match client {
        Some(c) => c,
        None => {
            return (
                false,
                format!("[ERROR] Provider not found: {}", provider_alias),
            );
        }
    };

    // --- VERIFICATION TOOL SCHEMA ---
    let verify_tool = json!({
        "name": "verify_security_intent",
        "description": "Report the security analysis of a proposed tool call.",
        "parameters": {
            "type": "object",
            "properties": {
                "safe": {
                    "type": "boolean",
                    "description": "True if the tool call is safe and aligns with the user's original intent."
                },
                "confidence": {
                    "type": "number",
                    "description": "Confidence level of the analysis (0.0 to 1.0)."
                },
                "reason": {
                    "type": "string",
                    "description": "Brief explanation of the decision."
                }
            },
            "required": ["safe", "confidence", "reason"]
        }
    });

    // --- STRUCTURED INPUT ---
    let mut user_content = format!("### USER INTENT:\n{}\n\n", user_prompt);

    if let Some(res) = last_tool_result {
        user_content.push_str(&format!("### LAST OBSERVATION:\n{}\n\n", res));
    }

    user_content.push_str(&format!(
        "### PROPOSED TOOL CALL:\n- Tool: {}\n- Arguments: {}\n\n\
        Analyze the security and intent alignment. Use the verify_security_intent tool to report your findings.",
        tool_name,
        serde_json::to_string(args).unwrap_or_default()
    ));

    let data = vec![DataSource {
        content: json!(user_content),
        content_type: "text/plain".to_string(),
        is_file_or_url: false,
        metadata: HashMap::new(),
    }];

    // Set system prompt on client state
    let system_prompt = "You are a Security Verifier. Analyze tool calls against user intent.";
    client.get_state_mut().system_prompt = Some(system_prompt.to_string());

    match client.send_as_verifier(data, verify_tool).await {
        Ok(v) => {
            let safe = v["safe"].as_bool().unwrap_or(false);
            let confidence = v["confidence"].as_f64().unwrap_or(1.0);
            let reason = v["reason"].as_str().unwrap_or("").to_string();

            if confidence < config.security.dual_llm_confidence_threshold {
                return (
                    false,
                    format!("[LOW_CONFIDENCE:{:.2}] {}", confidence, reason),
                );
            }
            (safe, reason)
        }
        Err(e) => (false, format!("[ERROR] Verification failed: {}", e)),
    }
}
