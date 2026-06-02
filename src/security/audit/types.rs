use serde::{Deserialize, Serialize};

/// Type-safe status for audit log entries, replacing free-form strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum AuditStatus {
    /// The tool call completed successfully.
    Success,
    /// The tool call failed with an error message.
    Failed(String),
    /// PQC encryption failed in high-security mode; entry stored with redacted args.
    PqcEncryptionFailed(String),
    /// PQC signing failed in high-security mode; entry stored without signature.
    IntegrityFailure(String),
    /// Entry was stored without a PQC signature because the private key was unavailable.
    SuccessWithoutSignature,
    /// Log rotation continuity marker (not a real tool call).
    #[doc(hidden)]
    LogRotationMarker,
}

impl std::fmt::Display for AuditStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuditStatus::Success => write!(f, "SUCCESS"),
            AuditStatus::Failed(reason) => write!(f, "FAILED: {reason}"),
            AuditStatus::PqcEncryptionFailed(reason) => {
                write!(f, "FAILED: {reason}; PQC_ENCRYPTION_FAILED")
            }
            AuditStatus::IntegrityFailure(reason) => {
                write!(f, "INTEGRITY_FAILURE: {reason}")
            }
            AuditStatus::SuccessWithoutSignature => {
                write!(f, "SUCCESS_WITHOUT_SIGNATURE: PQC private key unavailable")
            }
            AuditStatus::LogRotationMarker => write!(f, "CONTINUITY_MAINTAINED"),
        }
    }
}

impl TryFrom<String> for AuditStatus {
    type Error = String;

    fn try_from(s: String) -> Result<Self, String> {
        if s == "SUCCESS" {
            Ok(AuditStatus::Success)
        } else if let Some(reason) = s.strip_prefix("FAILED: ") {
            if let Some(inner) = reason.strip_suffix("; PQC_ENCRYPTION_FAILED") {
                Ok(AuditStatus::PqcEncryptionFailed(inner.to_string()))
            } else {
                Ok(AuditStatus::Failed(reason.to_string()))
            }
        } else if let Some(reason) = s.strip_prefix("INTEGRITY_FAILURE: ") {
            Ok(AuditStatus::IntegrityFailure(reason.to_string()))
        } else if s.starts_with("SUCCESS_WITHOUT_SIGNATURE") {
            Ok(AuditStatus::SuccessWithoutSignature)
        } else if s == "CONTINUITY_MAINTAINED" {
            Ok(AuditStatus::LogRotationMarker)
        } else {
            Ok(AuditStatus::Failed(s))
        }
    }
}

impl From<AuditStatus> for String {
    fn from(status: AuditStatus) -> String {
        status.to_string()
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct AuditEntry {
    pub timestamp: String,
    pub trace_id: String,
    pub subject: String,
    pub audience: String,
    pub model: String,
    pub provider: String,
    pub event_type: String,
    pub tool: String,
    pub args: serde_json::Value,
    pub pqc_confidential: bool,
    pub output: Option<String>,
    pub status: AuditStatus,
    pub exit_code: Option<i32>,
    pub prev_hash: String,
    pub hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pqc_signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pqc_algorithm: Option<String>,
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub cli_version: String,
}

pub struct AuditParams<'a> {
    pub event_type: &'a str,
    pub tool_name: &'a str,
    pub args: serde_json::Value,
    pub output: Option<&'a str>,
    pub exit_code: Option<i32>,
    pub error: Option<&'a str>,
    pub context: Option<&'a serde_json::Value>,
    pub config: &'a crate::config::models::AppConfig,
}

pub struct AuditParamsBuilder<'a> {
    params: AuditParams<'a>,
}

impl<'a> AuditParamsBuilder<'a> {
    #[must_use]
    pub fn new(
        event_type: &'a str,
        tool_name: &'a str,
        config: &'a crate::config::models::AppConfig,
    ) -> Self {
        Self {
            params: AuditParams {
                event_type,
                tool_name,
                args: serde_json::json!({}),
                output: None,
                exit_code: None,
                error: None,
                context: None,
                config,
            },
        }
    }

    #[must_use]
    pub fn args(mut self, args: serde_json::Value) -> Self {
        self.params.args = args;
        self
    }

    #[must_use]
    pub fn output(mut self, output: &'a str) -> Self {
        self.params.output = Some(output);
        self
    }

    #[must_use]
    pub fn exit_code(mut self, code: i32) -> Self {
        self.params.exit_code = Some(code);
        self
    }

    #[must_use]
    pub fn error(mut self, error: &'a str) -> Self {
        self.params.error = Some(error);
        self
    }

    #[must_use]
    pub fn context(mut self, context: &'a serde_json::Value) -> Self {
        self.params.context = Some(context);
        self
    }

    pub fn log(self) {
        crate::security::audit::logger::log_audit(self.params);
    }

    #[must_use]
    pub fn log_and_return(self, log_path: Option<&std::path::Path>) -> Option<AuditEntry> {
        crate::security::audit::logger::log_audit_and_return(self.params, log_path)
    }
}

impl<'a> AuditParams<'a> {
    #[must_use]
    pub fn builder(
        event_type: &'a str,
        tool_name: &'a str,
        config: &'a crate::config::models::AppConfig,
    ) -> AuditParamsBuilder<'a> {
        AuditParamsBuilder::new(event_type, tool_name, config)
    }
}
