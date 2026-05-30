use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Enum types for type-safe security configuration
// ---------------------------------------------------------------------------

/// Security level — controls PQC enforcement strictness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SecurityLevel {
    /// Strict PQC enforcement; high-risk actions without signatures are blocked.
    #[default]
    High,
    /// Permissive checks; warnings instead of blocks for interoperability.
    Standard,
}

impl SecurityLevel {
    /// Convert to the TOML string representation (for serialization).
    pub fn as_str(&self) -> &'static str {
        match self {
            SecurityLevel::High => "high",
            SecurityLevel::Standard => "standard",
        }
    }
}

impl std::fmt::Display for SecurityLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for SecurityLevel {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SecurityLevel {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        match s.as_str() {
            "high" => Ok(SecurityLevel::High),
            "standard" => Ok(SecurityLevel::Standard),
            other => Err(serde::de::Error::unknown_variant(
                other,
                &["high", "standard"],
            )),
        }
    }
}

impl TryFrom<&str> for SecurityLevel {
    type Error = String;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "high" => Ok(SecurityLevel::High),
            "standard" => Ok(SecurityLevel::Standard),
            other => Err(format!("unknown security level: '{}'", other)),
        }
    }
}

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
    /// Pager for long output.  `""` = disabled (default), `"auto"` = try less / built-in,
    /// or a specific command like `"less"`.
    #[serde(default)]
    pub pager: Option<String>,
}

fn default_true() -> bool {
    true
}
fn default_request_timeout() -> u64 {
    1800
}
fn default_command_timeout() -> u64 {
    300
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
    10000
}
fn default_max_output_chars() -> usize {
    100000
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
            pager: Some("auto".to_string()),
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
    pub last_used_provider: Option<String>,
    pub last_used_model: Option<String>,
    pub last_used_v_provider: Option<String>,
    pub last_used_v_model: Option<String>,
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
/// The committee uses an "any-flag" model: if **any** member returns NeedsApproval,
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
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CommitteeMemberConfig {
    /// The provider name (e.g. "ollama", "openrouter", "openai").
    pub provider: String,
    /// The model name (e.g. "gemma4:e2b", "gpt-4o", "claude-3-opus").
    pub model: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SecurityConfig {
    #[serde(default)]
    pub verifier_enabled: Option<bool>,
    #[serde(default = "default_unified_provider")]
    pub verifier_provider: String,
    #[serde(default = "default_verifier_model")]
    pub verifier_model: String,
    #[serde(default)]
    pub security_level: SecurityLevel,
    /// Additional verifier committee members beyond the primary (verifier_provider/model).
    ///
    /// When configured, the verifier runs ALL members (including the primary legacy pair)
    /// concurrently. If ANY member flags the call as NeedsApproval, human approval is required.
    /// Only if ALL members return Allowed is the call auto-approved.
    ///
    /// To use committee mode, add entries like:
    /// ```toml
    /// [security.verifier_committee]
    /// members = [
    ///   { provider = "openai", model = "gpt-4o" },
    ///   { provider = "anthropic", model = "claude-3-opus" },
    /// ]
    /// ```
    #[serde(default)]
    pub verifier_committee: VerifierCommitteeConfig,
}

fn default_verifier_model() -> String {
    "".to_string()
}
fn default_unified_provider() -> String {
    "".to_string()
}
/// Configuration for the verifier committee.
///
/// The committee runs multiple independent LLM verifiers concurrently.
/// The "any-flag" policy means: if ANY member flags a call as NeedsApproval,
/// human approval is required. Only if ALL members return Allowed is the call
/// auto-approved.
///
/// # Examples
///
/// ```toml
/// [security.verifier_committee]
/// members = [
///   { provider = "ollama", model = "gemma4:e2b" },
///   { provider = "openai", model = "gpt-4o-mini" },
/// ]
/// ```
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct VerifierCommitteeConfig {
    /// List of committee members (provider/model pairs).
    #[serde(default)]
    pub members: Vec<CommitteeMemberConfig>,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            verifier_enabled: None,
            verifier_provider: default_unified_provider(),
            verifier_model: default_verifier_model(),
            security_level: SecurityLevel::High,
            verifier_committee: VerifierCommitteeConfig::default(),
        }
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
    pub fn validate(&self) -> Vec<ValidationError> {
        let mut errors = Vec::new();

        // --- security_level ---
        // No runtime check needed: invalid values are rejected at
        // deserialization time by the SecurityLevel custom Deserialize impl.

        // --- cross-field: verifier_enabled enabled but nothing configured ---
        if self.verifier_enabled.unwrap_or(false) {
            let has_legacy = !self.verifier_provider.is_empty() && !self.verifier_model.is_empty();
            let has_committee = !self.verifier_committee.members.is_empty();
            if !has_legacy && !has_committee {
                errors.push(ValidationError {
                    field: "verifier_enabled".to_string(),
                    message: "verifier_enabled is enabled but neither legacy provider/model nor verifier_committee members are configured. Set verifier_provider/model or add verifier_committee.members.".to_string(),
                });
            }
            // Warn if legacy is partially configured
            if self.verifier_provider.is_empty() && !self.verifier_model.is_empty() {
                errors.push(ValidationError {
                    field: "verifier_provider".to_string(),
                    message: "verifier_provider is empty but verifier_model is set. Both or neither must be set.".to_string(),
                });
            }
            if !self.verifier_provider.is_empty() && self.verifier_model.is_empty() {
                errors.push(ValidationError {
                    field: "verifier_model".to_string(),
                    message: "verifier_model is empty but verifier_provider is set. Both or neither must be set.".to_string(),
                });
            }
        }

        // --- cross-field: committee members with empty provider/model ---
        for (idx, member) in self.verifier_committee.members.iter().enumerate() {
            if member.provider.is_empty() {
                errors.push(ValidationError {
                    field: format!("verifier_committee[{}].provider", idx),
                    message: "committee member provider must not be empty".to_string(),
                });
            }
            if member.model.is_empty() {
                errors.push(ValidationError {
                    field: format!("verifier_committee[{}].model", idx),
                    message: "committee member model must not be empty".to_string(),
                });
            }
        }

        errors
    }

    /// Returns advisory warnings for suboptimal (but not invalid) configuration
    /// combinations.  These do **not** block configuration loading but should
    /// be surfaced to the user (e.g., at startup or via `/info`).
    pub fn validate_warnings(&self) -> Vec<ValidationError> {
        let mut warnings = Vec::new();

        // --- cross-field: high security without verifier ---
        if self.security_level == SecurityLevel::High && !self.verifier_enabled.unwrap_or(false) {
            warnings.push(ValidationError {
                field: "security_level".to_string(),
                message: "security_level 'high' is set but verifier_enabled is not enabled — high-risk tools will escalate to Critical".to_string(),
            });
        }

        warnings
    }

    /// Convenience wrapper that returns `Ok(())` if valid, or `Err` with
    /// a human-readable summary of all validation failures (errors only).
    /// Use [`Self::validate_warnings`] separately for advisory issues.
    pub fn validate_or_err(&self) -> Result<(), String> {
        let errors = self.validate();
        if errors.is_empty() {
            Ok(())
        } else {
            let messages: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
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
