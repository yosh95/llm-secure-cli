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

/// Auto-approval level — controls which tool calls bypass human approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AutoApprovalLevel {
    /// All tools require manual approval (safest).
    #[default]
    None,
    /// Only low-risk tools are auto-approved.
    Low,
    /// Low and medium-risk tools are auto-approved.
    Medium,
}

impl AutoApprovalLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            AutoApprovalLevel::None => "none",
            AutoApprovalLevel::Low => "low",
            AutoApprovalLevel::Medium => "medium",
        }
    }
}

impl std::fmt::Display for AutoApprovalLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for AutoApprovalLevel {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for AutoApprovalLevel {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        match s.as_str() {
            "none" => Ok(AutoApprovalLevel::None),
            "low" => Ok(AutoApprovalLevel::Low),
            "medium" => Ok(AutoApprovalLevel::Medium),
            other => Err(serde::de::Error::unknown_variant(
                other,
                &["none", "low", "medium"],
            )),
        }
    }
}

/// Verifier fallback policy when the Dual LLM Verifier is unavailable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VerifierFallback {
    /// Force human approval for every tool call (fail-safe).
    #[default]
    RequireApproval,
    /// Block all tool calls when the verifier is down (maximum safety).
    Block,
}

impl VerifierFallback {
    pub fn as_str(&self) -> &'static str {
        match self {
            VerifierFallback::RequireApproval => "require_approval",
            VerifierFallback::Block => "block",
        }
    }
}

impl std::fmt::Display for VerifierFallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for VerifierFallback {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for VerifierFallback {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        match s.as_str() {
            "require_approval" => Ok(VerifierFallback::RequireApproval),
            "block" => Ok(VerifierFallback::Block),
            other => Err(serde::de::Error::unknown_variant(
                other,
                &["require_approval", "block"],
            )),
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
    #[serde(default = "default_max_security_log")]
    pub max_security_log_lines: usize,
    #[serde(default = "default_max_audit_log")]
    pub max_audit_log_lines: usize,
    #[serde(default = "default_max_audit_archives")]
    pub max_audit_archives: usize,
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
fn default_max_security_log() -> usize {
    1000
}
fn default_max_audit_log() -> usize {
    10000
}
fn default_max_audit_archives() -> usize {
    10
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
            max_security_log_lines: default_max_security_log(),
            max_audit_log_lines: default_max_audit_log(),
            max_audit_archives: default_max_audit_archives(),
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

#[derive(Serialize, Deserialize, Clone)]
pub struct SecurityConfig {
    #[serde(default = "default_allowed_paths")]
    pub allowed_paths: Vec<String>,
    #[serde(default)]
    pub blocked_paths: Vec<String>,
    #[serde(default)]
    pub high_risk_tools: Vec<String>,
    #[serde(default)]
    pub medium_risk_tools: Vec<String>,
    #[serde(default)]
    pub low_risk_tools: Vec<String>,
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(default = "default_true")]
    pub static_analysis_is_error: bool,
    #[serde(default)]
    pub scaling_patterns: Vec<String>,
    #[serde(default)]
    pub auto_approval_level: Option<AutoApprovalLevel>,
    #[serde(default)]
    pub dual_llm_verification: Option<bool>,
    #[serde(default = "default_unified_provider")]
    pub dual_llm_provider: String,
    #[serde(default = "default_dual_llm_model")]
    pub dual_llm_model: String,
    #[serde(default = "default_confidence_threshold")]
    pub dual_llm_confidence_threshold: f64,
    #[serde(default)]
    pub security_level: SecurityLevel,
    #[serde(default)]
    pub verifier_fallback: VerifierFallback,
}

fn default_allowed_paths() -> Vec<String> {
    vec![".".to_string()]
}
fn default_dual_llm_model() -> String {
    "".to_string()
}
fn default_unified_provider() -> String {
    "".to_string()
}
fn default_confidence_threshold() -> f64 {
    0.7
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            allowed_paths: default_allowed_paths(),
            blocked_paths: Vec::new(),
            high_risk_tools: Vec::new(),
            medium_risk_tools: Vec::new(),
            low_risk_tools: Vec::new(),
            allowed_tools: None,
            static_analysis_is_error: true,
            scaling_patterns: Vec::new(),
            auto_approval_level: None,
            dual_llm_verification: None,
            dual_llm_provider: default_unified_provider(),
            dual_llm_model: default_dual_llm_model(),
            dual_llm_confidence_threshold: default_confidence_threshold(),
            security_level: SecurityLevel::High,
            verifier_fallback: VerifierFallback::RequireApproval,
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

        // --- auto_approval_level ---
        // No runtime check needed: invalid values are rejected at
        // deserialization time by the AutoApprovalLevel custom Deserialize impl.

        // --- verifier_fallback ---
        // No runtime check needed: invalid values are rejected at
        // deserialization time by the VerifierFallback custom Deserialize impl.

        // --- dual_llm_confidence_threshold ---
        let threshold = self.dual_llm_confidence_threshold;
        if threshold <= 0.0 || threshold > 1.0 {
            errors.push(ValidationError {
                field: "dual_llm_confidence_threshold".to_string(),
                message: format!("must be in range (0.0, 1.0], got {}", threshold),
            });
        }

        // --- security_level ---
        // No runtime check needed: invalid values are rejected at
        // deserialization time by the SecurityLevel custom Deserialize impl.

        // --- allowed_paths must not be empty ---
        if self.allowed_paths.is_empty() {
            errors.push(ValidationError {
                field: "allowed_paths".to_string(),
                message: "must contain at least one path".to_string(),
            });
        }

        // --- cross-field: dual_llm_verification enabled but no provider ---
        if self.dual_llm_verification.unwrap_or(false) && self.dual_llm_provider.is_empty() {
            errors.push(ValidationError {
                field: "dual_llm_provider".to_string(),
                message: "dual_llm_verification is enabled but dual_llm_provider is empty"
                    .to_string(),
            });
        }

        // --- cross-field: dual_llm_verification enabled but no model ---
        if self.dual_llm_verification.unwrap_or(false) && self.dual_llm_model.is_empty() {
            errors.push(ValidationError {
                field: "dual_llm_model".to_string(),
                message: "dual_llm_verification is enabled but dual_llm_model is empty".to_string(),
            });
        }

        errors
    }

    /// Returns advisory warnings for suboptimal (but not invalid) configuration
    /// combinations.  These do **not** block configuration loading but should
    /// be surfaced to the user (e.g., at startup or via `/info`).
    pub fn validate_warnings(&self) -> Vec<ValidationError> {
        let mut warnings = Vec::new();

        // --- cross-field: high security without dual_llm ---
        if self.security_level == SecurityLevel::High
            && !self.dual_llm_verification.unwrap_or(false)
        {
            warnings.push(ValidationError {
                field: "security_level".to_string(),
                message: "security_level 'high' is set but dual_llm_verification is not enabled — high-risk tools will escalate to Critical".to_string(),
            });
        }

        // --- cross-field: auto_approval medium with dual_llm off ---
        if self.auto_approval_level == Some(AutoApprovalLevel::Medium)
            && !self.dual_llm_verification.unwrap_or(false)
        {
            warnings.push(ValidationError {
                field: "auto_approval_level".to_string(),
                message: "auto_approval_level 'medium' without dual_llm_verification is not recommended — high-risk tools lack semantic verification".to_string(),
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
    #[serde(default)]
    pub zero_trust: bool,
}

/// Post-Quantum Cryptography algorithm selection.
///
/// These settings control which NIST-standardized PQC algorithms the application
/// uses for digital signatures (ML-DSA) and key encapsulation (ML-KEM).
///
/// Currently supported:
/// - ML-DSA: "ML-DSA-87" (NIST Level 5, the highest security level)
/// - ML-KEM: "ML-KEM-1024" (NIST Level 5, the highest security level)
///
/// To change algorithms (e.g., for enterprise compliance), update these values
/// in config.toml and ensure the corresponding crate features are enabled.
#[derive(Serialize, Deserialize, Clone)]
pub struct PqcConfig {
    /// ML-DSA algorithm variant for digital signatures.
    /// Supported: "ML-DSA-87"
    #[serde(default = "default_ml_dsa_algorithm")]
    pub ml_dsa_algorithm: String,

    /// ML-KEM algorithm variant for key encapsulation.
    /// Supported: "ML-KEM-1024"
    #[serde(default = "default_ml_kem_algorithm")]
    pub ml_kem_algorithm: String,
}

fn default_ml_dsa_algorithm() -> String {
    "ML-DSA-87".to_string()
}

fn default_ml_kem_algorithm() -> String {
    "ML-KEM-1024".to_string()
}

impl Default for PqcConfig {
    fn default() -> Self {
        Self {
            ml_dsa_algorithm: default_ml_dsa_algorithm(),
            ml_kem_algorithm: default_ml_kem_algorithm(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub security: SecurityConfig,
    #[serde(default)]
    pub pqc: PqcConfig,
    #[serde(default)]
    pub brave_search: BraveSearchConfig,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
    #[serde(flatten)]
    pub providers: HashMap<String, ProviderConfig>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct BraveSearchConfig {
    /// Maximum number of search results to consider for context extraction (1–50).
    #[serde(default = "default_brave_count")]
    pub count: u64,
    /// Approximate maximum tokens in the returned context (1024–32768).
    #[serde(default = "default_brave_max_tokens")]
    pub max_tokens: u64,
    /// Maximum number of URLs in the response (1–50).
    #[serde(default = "default_brave_max_urls")]
    pub max_urls: u64,
    /// Relevance threshold: "strict", "balanced", "lenient", or "disabled".
    #[serde(default = "default_brave_context_threshold_mode")]
    pub context_threshold_mode: String,
    /// Freshness filter: "pd" (24h), "pw" (7d), "pm" (31d), "py" (365d), or a date range.
    #[serde(default = "default_brave_freshness")]
    pub freshness: String,
}

fn default_brave_count() -> u64 {
    50
}
fn default_brave_max_tokens() -> u64 {
    32768
}
fn default_brave_max_urls() -> u64 {
    50
}
fn default_brave_context_threshold_mode() -> String {
    "balanced".to_string()
}
fn default_brave_freshness() -> String {
    String::new()
}

impl Default for BraveSearchConfig {
    fn default() -> Self {
        Self {
            count: default_brave_count(),
            max_tokens: default_brave_max_tokens(),
            max_urls: default_brave_max_urls(),
            context_threshold_mode: default_brave_context_threshold_mode(),
            freshness: default_brave_freshness(),
        }
    }
}
