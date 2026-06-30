use crate::llm::base::LlmClient;
use crate::llm::models::DataSource;
use crate::security::policy::{SECURITY_CONSTITUTION, SecurityContext};
use serde_json::{Value, json};

/// Verifier implements the "Tool Call Security Verification" logic.
/// It uses a secondary LLM to judge if a tool call needs human review.
///
/// NOTE: The Verifier LLM must NOT be configured with any tools itself
/// to prevent secondary prompt injection risks.
pub struct Verifier {
    verifier_llm: Box<dyn LlmClient>,
}

#[derive(Debug, PartialEq)]
pub enum VerificationResult {
    /// The verifier approved the tool call — safe to execute.
    Allowed,
    /// The verifier flagged the tool call as potentially unsafe or ambiguous.
    /// A human must review and decide.
    NeedsApproval(String),
    /// The verifier was unavailable (network error, API failure, etc.).
    /// A human must judge.
    FallbackRequired(String),
    /// The verifier encountered an error during processing.
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

/// Outcome of a verifier committee verification attempt.
#[derive(Clone, Debug)]
pub enum VerificationOutcome {
    /// The verifier explicitly approved the tool call — safe to auto-approve.
    Allowed(String),
    /// The verifier flagged the tool call as potentially unsafe.
    /// A human must review the explanation and decide whether to allow execution.
    NeedsApproval(String),
    /// The verifier was unavailable (network error, API failure, etc.).
    /// A human must judge — this is neither a pass nor a block.
    FallbackRequired(String),
}

/// Validates a tool call using a secondary LLM.
/// Returns true if safe, false if blocked or error.
pub fn verify_tool_call(
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
    });

    match outcome {
        VerificationOutcome::Allowed(_) => true,
        VerificationOutcome::NeedsApproval(_) => false,
        VerificationOutcome::FallbackRequired(_) => false,
    }
}

/// Validates a tool call using a secondary LLM and returns the full outcome.
/// The caller should handle `NeedsApproval` and `FallbackRequired` by requiring human approval.
pub fn verify_tool_call_full(params: VerificationParams<'_>) -> VerificationOutcome {
    let p = match &params.provider {
        Some(p) if !p.is_empty() => p.clone(),
        _ => {
            return VerificationOutcome::FallbackRequired(
                "Verifier not configured. Use /verifier add <provider:model> to add committee members, or use /verifier add at runtime."
                    .to_string(),
            );
        }
    };
    let m = match &params.model {
        Some(m) if !m.is_empty() => m.clone(),
        _ => {
            return VerificationOutcome::FallbackRequired(
                "Verifier not configured. Use /verifier add <provider:model> to add committee members, or use /verifier add at runtime."
                    .to_string(),
            );
        }
    };

    let client = {
        let registry = params
            .ctx_app
            .client_registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        registry.create_client(&p, &m, false, true, &params.ctx_app.config_manager)
    };

    let client = match client {
        Some(c) => c,
        None => {
            return VerificationOutcome::FallbackRequired(format!(
                "Could not create verifier client for {p}/{m}: the verifier is unavailable."
            ));
        }
    };

    let ctx = params.context.unwrap_or_else(SecurityContext::gather);

    let mut verifier = Verifier::new(client);
    match verifier.verify(params.tool_name, params.tool_args, &ctx) {
        VerificationResult::Allowed => VerificationOutcome::Allowed("Allowed".to_string()),
        VerificationResult::NeedsApproval(reason) => VerificationOutcome::NeedsApproval(reason),
        VerificationResult::FallbackRequired(reason) => {
            VerificationOutcome::FallbackRequired(reason)
        }
        VerificationResult::Error(e) => {
            VerificationOutcome::FallbackRequired(format!("Verifier unavailable: {e}"))
        }
    }
}

/// Parses the raw text response from the verifier LLM into a `VerificationResult`.
///
/// The verifier LLM must reply with exactly:
///   ALLOW
/// or
///   REVIEW: `<reason>`
///
/// However, we also accept `REVIEW` without a colon (e.g. "REVIEW dangerous")
/// for robustness with smaller LLMs that may omit the colon.
///
/// Parsing logic:
///   - First word is "ALLOW" (case-insensitive) → Allowed
///   - First word is "REVIEW" (case-insensitive) → NeedsApproval
///     with the rest of the line as the reason
///   - Anything else → NeedsApproval (safety-first)
#[must_use]
pub fn parse_verifier_response(response: &str) -> VerificationResult {
    let first_line = response.lines().next().unwrap_or("").trim();
    if first_line.is_empty() {
        return VerificationResult::NeedsApproval(
            "Invalid verifier response. First line was empty.".to_string(),
        );
    }

    // Split into first word and the remainder, treating colon as a separator too
    let mut parts = first_line.splitn(2, |c: char| c.is_whitespace() || c == ':');
    let first_word = parts.next().unwrap_or("").trim();

    if first_word.eq_ignore_ascii_case("ALLOW") {
        return VerificationResult::Allowed;
    }

    if first_word.eq_ignore_ascii_case("REVIEW") {
        let reason = parts.next().map(|s| s.trim()).unwrap_or("").to_string();
        let reason = if reason.is_empty() {
            "Needs human review".to_string()
        } else {
            reason
        };
        return VerificationResult::NeedsApproval(reason);
    }

    VerificationResult::NeedsApproval(format!(
        "Invalid verifier response. First line was: {first_line}"
    ))
}

/// Hardcoded system-prompt template for the verifier.
///
/// This is deliberately not configurable by the user.
/// Placeholders are filled at verification time:
///   {constitution}, {security_context}
pub const VERIFIER_SYSTEM_PROMPT_TEMPLATE: &str = concat!(
    "{constitution}

",
    "## CURRENT SECURITY CONTEXT
",
    "```json
",
    "{security_context}
",
    "```",
);

/// Hardcoded user-prompt template for the verifier.
///
/// Deliberately not configurable.
/// Placeholders: {tool_name}, {tool_args}
pub const VERIFIER_USER_PROMPT_TEMPLATE: &str = concat!(
    "### Proposed tool call
",
    "Tool: {tool_name}

",
    "Arguments:
",
    "{tool_args}
",
    "
",
    "Example valid replies:
",
    "  ALLOW
",
    "  REVIEW: This modifies system files
",
    "  REVIEW This looks suspicious
",
    "
",
    "Reply with exactly one line: ALLOW or REVIEW <reason>",
);

impl Verifier {
    #[must_use]
    pub fn new(llm: Box<dyn LlmClient>) -> Self {
        Self { verifier_llm: llm }
    }

    pub fn verify(
        &mut self,
        tool_name: &str,
        tool_args: &Value,
        context: &SecurityContext,
    ) -> VerificationResult {
        let security_context_json = serde_json::to_string_pretty(context).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Failed to serialize SecurityContext for verifier prompt");
            format!("{{\"error\": \"SecurityContext serialization failed: {e}\"}}")
        });

        // Build the system prompt from the hardcoded template
        let system_prompt = VERIFIER_SYSTEM_PROMPT_TEMPLATE
            .replace("{constitution}", SECURITY_CONSTITUTION)
            .replace("{security_context}", &security_context_json);

        // Build the user prompt from the hardcoded template
        let tool_args_pretty = serde_json::to_string_pretty(tool_args).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Failed to serialize tool_args for verifier prompt");
            format!("{{\"error\": \"tool_args serialization failed: {e}\"}}")
        });
        let user_prompt = VERIFIER_USER_PROMPT_TEMPLATE
            .replace("{tool_name}", tool_name)
            .replace("{tool_args}", &tool_args_pretty);

        // Configure client for a one-off verification
        self.verifier_llm.get_state_mut().conversation.clear();
        self.verifier_llm.get_state_mut().system_prompt = Some(system_prompt);

        let data = vec![DataSource {
            content: json!(user_prompt),
            content_type: "text/plain".to_string(),
            is_file_or_url: false,
            metadata: std::collections::HashMap::new(),
        }];

        // Use the standard send method instead of send_as_verifier to support models without tool calling.
        match self.verifier_llm.send_verifier(data) {
            Ok(response_struct) => {
                let response = response_struct.content.unwrap_or_default();
                parse_verifier_response(&response)
            }
            Err(e) => VerificationResult::Error(format!("Verifier LLM error: {e}")),
        }
    }
}
