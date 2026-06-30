use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::defaults;

// ── GeneralConfig ─────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
pub struct GeneralConfig {
    pub system_prompt: Option<String>,
    #[serde(default = "default_request_timeout")]
    pub request_timeout: u64,
    #[serde(default = "default_verifier_timeout")]
    pub verifier_timeout: u64,
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

#[inline]
const fn default_request_timeout() -> u64 {
    defaults::DEFAULT_REQUEST_TIMEOUT
}
#[inline]
const fn default_verifier_timeout() -> u64 {
    defaults::DEFAULT_VERIFIER_TIMEOUT
}
#[inline]
const fn default_python_timeout() -> u64 {
    defaults::DEFAULT_PYTHON_TIMEOUT
}
#[inline]
const fn default_max_audit_log() -> usize {
    defaults::DEFAULT_MAX_AUDIT_LOG_LINES
}
#[inline]
const fn default_max_chat_log() -> usize {
    defaults::DEFAULT_MAX_CHAT_LOG_LINES
}
#[inline]
const fn default_max_chat_archives() -> usize {
    defaults::DEFAULT_MAX_CHAT_ARCHIVES
}
#[inline]
fn default_image_save_path() -> String {
    defaults::DEFAULT_IMAGE_SAVE_PATH.to_string()
}
#[inline]
const fn default_max_output_lines() -> usize {
    defaults::DEFAULT_MAX_OUTPUT_LINES
}
#[inline]
const fn default_max_output_chars() -> usize {
    defaults::DEFAULT_MAX_OUTPUT_CHARS
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            system_prompt: None,
            request_timeout: default_request_timeout(),
            verifier_timeout: default_verifier_timeout(),
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

// ── ProviderConfig ────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct ProviderConfig {
    pub api_key: Option<String>,
    pub api_url: Option<String>,
}

// ── AppState ──────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct AppState {
    /// Provider:model string (e.g. "openai:gpt-4o").
    /// Unified field replacing the old separate last_used_provider + last_used_model.
    pub last_model: Option<String>,
    /// Verifier committee members (provider:model strings).
    /// Managed at runtime via `/verifier add|delete` commands.
    /// On startup, falls back to `security.verifier_committee` (compile-time default)
    /// if this list is empty.
    #[serde(default)]
    pub verifier_committee: Vec<String>,
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
/// The `security.verifier_committee` field serves as a **fallback**
/// when state.toml has no runtime-configured members.
///
/// # Default
///
/// When neither runtime members nor the fallback `verifier_committee` are set,
/// the verifier falls back to manual human approval for all tool calls.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct SecurityConfig {
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

// ── PqcConfig ─────────────────────────────────────────────────────────────

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

#[inline]
fn default_pqc_signature_variant() -> String {
    defaults::DEFAULT_SIGNATURE_VARIANT.to_string()
}

#[inline]
fn default_pqc_kem_variant() -> String {
    defaults::DEFAULT_KEM_VARIANT.to_string()
}

impl Default for PqcConfig {
    fn default() -> Self {
        Self {
            signature_variant: default_pqc_signature_variant(),
            kem_variant: default_pqc_kem_variant(),
        }
    }
}

// ── AppConfig ─────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
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

impl Default for AppConfig {
    /// Creates an `AppConfig` with all fields defaulted.
    ///
    /// Built-in providers (`ollama`, `openrouter`, `vllm`, `openai`) are
    /// pre-populated with their default API URLs from [`defaults`].
    /// API keys are **never** stored here — they come from env vars / `.env`.
    fn default() -> Self {
        let mut providers = HashMap::new();
        providers.insert(
            "ollama".to_string(),
            ProviderConfig {
                api_key: None,
                api_url: Some(defaults::DEFAULT_OLLAMA_API_URL.to_string()),
            },
        );
        providers.insert(
            "openrouter".to_string(),
            ProviderConfig {
                api_key: None,
                api_url: Some(defaults::DEFAULT_OPENROUTER_API_URL.to_string()),
            },
        );
        providers.insert(
            "vllm".to_string(),
            ProviderConfig {
                api_key: None,
                api_url: Some(defaults::DEFAULT_VLLM_API_URL.to_string()),
            },
        );
        providers.insert(
            "openai".to_string(),
            ProviderConfig {
                api_key: None,
                api_url: Some(defaults::DEFAULT_OPENAI_API_URL.to_string()),
            },
        );

        Self {
            general: GeneralConfig::default(),
            pqc: PqcConfig::default(),
            security: SecurityConfig::default(),
            providers,
        }
    }
}

// ── CliOverrides ──────────────────────────────────────────────────────────
// This struct collects all CLI-provided overrides. The ConfigManager
// applies these on top of the defaults (CLI args > defaults).

/// Values parsed from the command line that override the file-based config.
#[derive(Clone, Debug, Default)]
pub struct CliOverrides {
    // General
    pub request_timeout: Option<u64>,
    pub verifier_timeout: Option<u64>,
    pub python_timeout: Option<u64>,
    pub image_save_path: Option<String>,
    pub max_audit_log_lines: Option<usize>,
    pub max_chat_log_lines: Option<usize>,
    pub max_chat_archives: Option<usize>,
    pub max_output_lines: Option<usize>,
    pub max_output_chars: Option<usize>,

    // PQC
    pub signature_variant: Option<String>,
    pub kem_variant: Option<String>,
}

impl CliOverrides {
    /// Apply these overrides to an existing `AppConfig`, producing a new one.
    #[must_use]
    pub fn apply_to(self, config: AppConfig) -> AppConfig {
        AppConfig {
            general: GeneralConfig {
                request_timeout: self
                    .request_timeout
                    .unwrap_or(config.general.request_timeout),
                verifier_timeout: self
                    .verifier_timeout
                    .unwrap_or(config.general.verifier_timeout),
                python_timeout: self.python_timeout.unwrap_or(config.general.python_timeout),
                image_save_path: self
                    .image_save_path
                    .unwrap_or(config.general.image_save_path),
                max_audit_log_lines: self
                    .max_audit_log_lines
                    .unwrap_or(config.general.max_audit_log_lines),
                max_chat_log_lines: self
                    .max_chat_log_lines
                    .unwrap_or(config.general.max_chat_log_lines),
                max_chat_archives: self
                    .max_chat_archives
                    .unwrap_or(config.general.max_chat_archives),
                max_output_lines: self
                    .max_output_lines
                    .unwrap_or(config.general.max_output_lines),
                max_output_chars: self
                    .max_output_chars
                    .unwrap_or(config.general.max_output_chars),
                ..config.general
            },
            pqc: PqcConfig {
                signature_variant: self
                    .signature_variant
                    .unwrap_or(config.pqc.signature_variant),
                kem_variant: self.kem_variant.unwrap_or(config.pqc.kem_variant),
            },
            security: SecurityConfig { ..config.security },
            ..config
        }
    }
}
