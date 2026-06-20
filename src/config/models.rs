use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Clone)]
pub struct GeneralConfig {
    pub system_prompt: Option<String>,
    #[serde(default = "default_request_timeout")]
    pub request_timeout: u64,
    #[serde(default = "default_python_timeout")]
    pub python_timeout: u64,
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

fn default_request_timeout() -> u64 {
    1800
}
fn default_python_timeout() -> u64 {
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
            request_timeout: default_request_timeout(),
            python_timeout: default_python_timeout(),
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
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct AppState {
    /// Provider:model string (e.g. "openai:gpt-4o").
    /// Unified field replacing the old separate last_used_provider + last_used_model.
    pub last_model: Option<String>,
    /// Verifier committee members (provider:model strings).
    /// Managed at runtime via `/verifier add|delete` commands.
    /// On startup, falls back to `security.verifier_committee` in config.toml
    /// if this list is empty.
    #[serde(default)]
    pub verifier_committee: Vec<String>,
    // (Removed: show_tool_result — tool output is now always displayed)
}

/// The verifier committee.
///
/// Each committee member is an independent LLM that evaluates tool call safety.
/// The committee uses a strict **any-flag** model: members are checked **one at
/// a time, in order**, and the **first** member that flags the tool call
/// (`NeedsApproval` or `FallbackRequired`) immediately hands off to
/// human-in-the-loop approval. Only if **all** members approve is the call
/// auto-approved.
///
/// Committee members can be managed at runtime via:
///   `/verifier add <provider:model>`   — adds a member (persisted to state.toml)
///   `/verifier delete <provider:model>` — removes a member
///   `/verifier list`                    — lists current members
///
/// The `security.verifier_committee` in config.toml serves as a **fallback**
/// when state.toml has no runtime-configured members.
///
/// # Default
///
/// When neither runtime members nor config.toml `verifier_committee` are set,
/// the verifier falls back to manual human approval for all tool calls.
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
    /// Used as a FALLBACK when state.toml has no runtime-configured members
    /// (managed via `/verifier add|delete`).
    /// When empty, the verifier falls back to manual human approval.
    ///
    /// Members are evaluated sequentially under a strict any-flag policy: the
    /// first member to flag the call hands off to human approval immediately.
    #[serde(default)]
    pub verifier_committee: Vec<String>,
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
                "Security config validation failed:
  - {}",
                messages.join(
                    "
  - "
                )
            ))
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct PqcConfig {
    /// PQC signature variant (ML-DSA).
    /// "ml-dsa-44" (lowest), "ml-dsa-65" (medium), or "ml-dsa-87" (highest).
    #[serde(default = "default_pqc_signature_variant")]
    pub signature_variant: String,

    /// PQC KEM variant (ML-KEM).
    /// "ml-kem-512" (lowest), "ml-kem-768" (medium), or "ml-kem-1024" (highest).
    #[serde(default = "default_pqc_kem_variant")]
    pub kem_variant: String,
}

fn default_pqc_signature_variant() -> String {
    "ml-dsa-44".to_string()
}

fn default_pqc_kem_variant() -> String {
    "ml-kem-512".to_string()
}

impl Default for PqcConfig {
    fn default() -> Self {
        Self {
            signature_variant: default_pqc_signature_variant(),
            kem_variant: default_pqc_kem_variant(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub pqc: PqcConfig,
    #[serde(default)]
    pub security: SecurityConfig,
    #[serde(default)]
    #[serde(flatten)]
    pub providers: HashMap<String, ProviderConfig>,
}
