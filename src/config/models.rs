use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Clone)]
pub struct GeneralConfig {
    pub system_prompt: Option<String>,
    #[serde(default = "default_true")]
    pub pdf_as_base64: bool,
    #[serde(default = "default_request_timeout")]
    pub request_timeout: u64,
    #[serde(default = "default_command_timeout")]
    pub command_timeout: u64,
    #[serde(default = "default_max_audit_log")]
    pub max_audit_log_lines: usize,
    #[serde(default = "default_max_chat_log")]
    pub max_chat_log_lines: usize,
    #[serde(default = "default_max_chat_archives")]
    pub max_chat_archives: usize,
    #[serde(default = "default_image_save_path")]
    pub image_save_path: String,
    #[serde(default = "default_max_output_lines")]
    pub max_output_lines: usize,
    #[serde(default = "default_max_output_chars")]
    pub max_output_chars: usize,
}

fn default_true() -> bool {
    true
}
fn default_request_timeout() -> u64 {
    1800
}
fn default_command_timeout() -> u64 {
    3600
}
fn default_max_audit_log() -> usize {
    10000
}
fn default_max_chat_log() -> usize {
    5000
}
fn default_max_chat_archives() -> usize {
    5
}
fn default_image_save_path() -> String {
    "~/Pictures/llsc".to_string()
}
fn default_max_output_lines() -> usize {
    5000
}
fn default_max_output_chars() -> usize {
    50000
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            system_prompt: None,
            pdf_as_base64: default_true(),
            request_timeout: default_request_timeout(),
            command_timeout: default_command_timeout(),
            max_audit_log_lines: default_max_audit_log(),
            max_chat_log_lines: default_max_chat_log(),
            max_chat_archives: default_max_chat_archives(),
            image_save_path: default_image_save_path(),
            max_output_lines: default_max_output_lines(),
            max_output_chars: default_max_output_chars(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct ProviderConfig {
    pub api_key: Option<String>,
    pub api_url: Option<String>,
    /// Payload formatter hint for OpenAI-compatible providers.
    ///
    /// - `"high_feature"` — Use the Anthropic/Gemini-compatible formatter
    ///   (native PDF documents, extended vision support).
    /// - `"generic"`      — Use the standard OpenAI-compatible formatter.
    /// - `None` (omitted) — Auto-detect from the model name (legacy behaviour,
    ///   kept as a fallback for backwards compatibility).
    pub formatter: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct AppState {
    /// Provider:model string (e.g. "openai:gpt-4o").
    /// Unified field replacing the old separate last_used_provider + last_used_model.
    pub last_model: Option<String>,
    /// Verifier committee members (provider:model strings).
    /// DEPRECATED: Use SecurityConfig.verifier_committee in config.toml.
    /// Kept for backward compatibility during migration.
    #[serde(default)]
    pub verifier_committee_members: Vec<String>,
    /// Whether to display tool execution results to the user.
    /// Default (None/false) = hidden (not shown).
    #[serde(default)]
    pub show_tool_result: Option<bool>,
    #[serde(default)]
    pub model_aliases: HashMap<String, ModelAlias>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ModelAlias {
    pub target: String,
}

/// A single member of the verifier committee.
///
/// Each committee member is an independent LLM that evaluates tool call safety.
/// The committee uses an "any-flag" model: if **any** member returns `NeedsApproval`,
/// the call requires human approval. Only if **all** members return Allowed is the
/// call auto-approved.
///
/// # Backward Compatibility
///
/// If `verifier_provider` and `verifier_model` are set (the legacy single-verifier
/// config), that pair is treated as the first committee member. Additional members
/// can be added via `verifier_committee` without removing the legacy fields.
///
/// # Default
///
/// When neither legacy fields nor `verifier_committee` are configured, the verifier
/// is disabled and falls back to manual human approval for all tool calls.

#[derive(Serialize, Deserialize, Clone)]
pub struct SecurityConfig {
    /// When true, all Y/n and feedback prompts are automatically answered Yes.
    /// Equivalent to the old `LLM_SECURE_AUTO_APPROVE` env var.
    /// WARNING: This bypasses all user confirmation — use with extreme caution.
    #[serde(default)]
    pub auto_approve: bool,

    /// Master switch for the Verifier Committee.
    /// When true (default), tool calls are verified by the configured committee.
    /// When false, all tool calls fall through to manual human approval.
    #[serde(default = "default_verifier_enabled")]
    pub verifier_enabled: bool,

    /// Verifier Committee members (provider:model strings, e.g. "openai:gpt-4o").
    /// When empty, the verifier falls back to manual human approval.
    #[serde(default)]
    pub verifier_committee: Vec<String>,

    /// Committee voting policy.
    ///
    /// - `"any-flag"` (conservative): ANY member flags → human approval required.
    /// - `"majority"` (balanced, **default**): majority vote decides.
    #[serde(default)]
    pub committee_policy: CommitteePolicy,
}

fn default_verifier_enabled() -> bool {
    true
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            auto_approve: false,
            verifier_enabled: true,
            verifier_committee: Vec::new(),
            committee_policy: CommitteePolicy::default(),
        }
    }
}

/// Committee voting policy for the Verifier Committee.
///
/// Determines how individual LLM verdicts are aggregated into a single decision.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum CommitteePolicy {
    /// Any-flag policy (conservative):
    /// If ANY member returns NeedsApproval → human approval is required.
    /// Only if ALL members return Allowed → the call is auto-approved.
    /// This minimizes false negatives (missed attacks) at the cost of
    /// more false positives (unnecessary human review).
    AnyFlag,
    /// Majority-vote policy (balanced, DEFAULT):
    /// Decisions are aggregated by simple majority.
    /// - Allowed votes >= (total / 2 + 1) → Allowed
    /// - NeedsApproval votes >= (total / 2 + 1) → NeedsApproval
    /// - Otherwise → NeedsApproval (ties/default to human review)
    /// This reduces false positives compared to any-flag while maintaining
    /// strong security through diverse LLM perspectives.
    Majority,
}

impl Default for CommitteePolicy {
    fn default() -> Self {
        Self::Majority
    }
}

/// Describes a single validation failure in a [`SecurityConfig`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub field: String,
    pub message: String,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

impl std::error::Error for ValidationError {}

impl SecurityConfig {
    /// Validates the security configuration and returns a list of all
    /// validation **errors** found. An empty vec means the config has no
    /// hard-validation errors.
    ///
    /// Use [`Self::validate_warnings`] for advisory issues that should not
    /// block configuration loading but are worth surfacing to the user.
    #[must_use]
    pub fn validate(&self) -> Vec<ValidationError> {
        Vec::new()
    }

    /// Returns advisory warnings for suboptimal (but not invalid) configuration
    /// combinations.  These do **not** block configuration loading but should
    /// be surfaced to the user (e.g., at startup or via `/info`).
    #[must_use]
    pub fn validate_warnings(&self) -> Vec<ValidationError> {
        Vec::new()
    }

    /// Convenience wrapper that returns `Ok(())` if valid, or `Err` with
    /// a human-readable summary of all validation failures (errors only).
    /// Use [`Self::validate_warnings`] separately for advisory issues.
    pub fn validate_or_err(&self) -> Result<(), String> {
        let errors = self.validate();
        if errors.is_empty() {
            Ok(())
        } else {
            let messages: Vec<String> = errors
                .iter()
                .map(std::string::ToString::to_string)
                .collect();
            Err(format!(
                "Security config validation failed:\n  - {}",
                messages.join("\n  - ")
            ))
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Transport type: "stdio" (default) or "streamable-http"
    #[serde(default)]
    pub transport: String,
    /// Base URL for Streamable HTTP transport (e.g., "<https://example.com/mcp>")
    #[serde(default)]
    pub api_url: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub security: SecurityConfig,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
    #[serde(flatten)]
    pub providers: HashMap<String, ProviderConfig>,
}
