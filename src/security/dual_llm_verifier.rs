use crate::llm::base::LlmClient;
use crate::llm::models::DataSource;
use crate::security::policy::{SECURITY_CONSTITUTION, SecurityContext};
use serde_json::{Value, json};

/// DualLLMVerifier implements the "Intent and Policy Verification" logic.
/// It uses a secondary LLM to judge if a tool call is safe based on the user's intent
/// and the system's hardcoded Security Constitution.
///
/// NOTE: The Verifier LLM must NOT be configured with any tools itself (except the verdict tool)
/// to prevent secondary prompt injection risks.
pub struct DualLLMVerifier {
    verifier_llm: Box<dyn LlmClient>,
}

#[derive(PartialEq)]
pub enum VerificationResult {
    Allowed,
    Rejected(String),
    /// The verifier was unavailable (network error, API failure, etc.).
    /// The tool call cannot be automatically verified — a human must judge.
    FallbackRequired(String),
    Error(String),
}

pub struct VerificationParams<'a> {
    pub ctx_app: std::sync::Arc<crate::core::context::AppContext>,
    pub user_query: &'a str,
    pub tool_name: &'a str,
    pub tool_args: &'a Value,
    pub context: Option<SecurityContext>,
    pub config: &'a crate::config::models::SecurityConfig,
    pub provider: Option<String>,
    pub model: Option<String>,
}

/// Outcome of a Dual LLM verification attempt.
/// Distinguishes between definitive safety judgments and cases where
/// the verifier was unavailable and a human must decide.
#[derive(Clone)]
pub enum VerificationOutcome {
    /// The verifier explicitly approved the tool call.
    Allowed(String),
    /// The verifier explicitly rejected the tool call (policy violation).
    Rejected(String),
    /// The verifier was unavailable (network error, API failure, etc.).
    /// A human must judge — this is neither a pass nor a block.
    FallbackRequired(String),
}

/// Validates a tool call using a secondary LLM.
/// Returns true if safe, false if blocked or error.
/// NOTE: This simplified API cannot distinguish FallbackRequired from Rejected.
/// Use verify_tool_call_full for the complete outcome.
pub async fn verify_tool_call(
    ctx_app: std::sync::Arc<crate::core::context::AppContext>,
    user_query: &str,
    tool_name: &str,
    tool_args: &Value,
    context: Option<SecurityContext>,
    config: &crate::config::models::SecurityConfig,
) -> bool {
    let outcome = verify_tool_call_full(VerificationParams {
        ctx_app,
        user_query,
        tool_name,
        tool_args,
        context,
        config,
        provider: None,
        model: None,
    })
    .await;

    match outcome {
        VerificationOutcome::Allowed(_) => true,
        VerificationOutcome::Rejected(_) => false,
        VerificationOutcome::FallbackRequired(_) => false,
    }
}

/// Validates a tool call using a secondary LLM and returns the full outcome.
/// The caller should handle FallbackRequired by requiring human approval,
/// rather than treating it as a simple block.
pub async fn verify_tool_call_full(params: VerificationParams<'_>) -> VerificationOutcome {
    let p = params
        .provider
        .unwrap_or_else(|| params.config.dual_llm_provider.clone());
    let m = params
        .model
        .unwrap_or_else(|| params.config.dual_llm_model.clone());

    let client = {
        let registry = params.ctx_app.client_registry.lock().await;
        registry.create_client(&p, &m, false, true, &params.ctx_app.config_manager)
    };

    let client = match client {
        Some(c) => c,
        None => {
            // Verifier client creation failed — likely a configuration or
            // network issue. We cannot determine intent, so a human must judge.
            return VerificationOutcome::FallbackRequired(format!(
                "Could not create verifier client for {}/{}: the verifier is unavailable.",
                p, m
            ));
        }
    };

    let ctx = params
        .context
        .unwrap_or_else(|| SecurityContext::gather(&params.config.security_level));

    let mut verifier = DualLLMVerifier::new(client);
    match verifier
        .verify(params.user_query, params.tool_name, params.tool_args, &ctx)
        .await
    {
        VerificationResult::Allowed => VerificationOutcome::Allowed("Allowed".to_string()),
        VerificationResult::Rejected(reason) => VerificationOutcome::Rejected(reason),
        VerificationResult::FallbackRequired(reason) => {
            VerificationOutcome::FallbackRequired(reason)
        }
        // LLM call itself failed (timeout, network error, etc.).
        // The verifier couldn't produce a judgment — fall back to human.
        VerificationResult::Error(e) => {
            VerificationOutcome::FallbackRequired(format!("Verifier unavailable: {}", e))
        }
    }
}

impl DualLLMVerifier {
    pub fn new(llm: Box<dyn LlmClient>) -> Self {
        Self { verifier_llm: llm }
    }

    pub async fn verify(
        &mut self,
        user_query: &str,
        tool_name: &str,
        tool_args: &Value,
        context: &SecurityContext,
    ) -> VerificationResult {
        // Build the semantic security check prompt
        let system_prompt = format!(
            "{}\n\n## CURRENT SECURITY CONTEXT\n```json\n{}\n```",
            SECURITY_CONSTITUTION,
            serde_json::to_string_pretty(context).unwrap_or_default()
        );

        let user_prompt = format!(
            "### UNTRUSTED USER INPUT (CONTEXT ONLY)\n<user_intent>\n{}\n</user_intent>\n\n### PROPOSED TOOL CALL\n<tool_call>\nTool: {}\nArguments: {}\n</tool_call>\n\nEvaluation Task: Does the tool_call align with user_intent without violating the Security Constitution?",
            user_query,
            tool_name,
            serde_json::to_string_pretty(tool_args).unwrap_or_default()
        );

        // Define the structured tool schema for the verdict
        let verify_tool_schema = json!({
            "name": "submit_verdict",
            "description": "Submit the security evaluation result for a tool call.",
            "parameters": {
                "type": "object",
                "properties": {
                    "decision": {
                        "type": "string",
                        "enum": ["ALLOW", "BLOCK"],
                        "description": "The security decision. ALLOW if safe, BLOCK if risky or violates policy."
                    },
                    "reason": {
                        "type": "string",
                        "description": "A detailed explanation of the decision, referencing specific policy violations if blocked."
                    }
                },
                "required": ["decision", "reason"]
            }
        });

        // Configure client for a one-off verification
        self.verifier_llm.get_state_mut().conversation.clear();
        self.verifier_llm.get_state_mut().system_prompt = Some(system_prompt);
        self.verifier_llm.get_state_mut().system_prompt_enabled = true;

        let data = vec![DataSource {
            content: json!(user_prompt),
            content_type: "text/plain".to_string(),
            is_file_or_url: false,
            metadata: std::collections::HashMap::new(),
        }];

        // Use the specialized structured output path
        match self
            .verifier_llm
            .send_as_verifier(data, verify_tool_schema)
            .await
        {
            Ok(args) => {
                let decision = args.get("decision").and_then(|v| v.as_str()).unwrap_or("");
                let reason = args
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("No reason provided");

                if decision == "ALLOW" {
                    VerificationResult::Allowed
                } else {
                    VerificationResult::Rejected(reason.to_string())
                }
            }
            Err(e) => VerificationResult::Error(format!("Verifier LLM error: {}", e)),
        }
    }
}
