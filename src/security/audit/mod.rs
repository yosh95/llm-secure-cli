//! Audit log module -- cryptographically chained audit trail with
//! PQC encryption and integrity verification.

pub mod chain;
pub mod logger;
pub mod rotation;
pub mod types;

// Re-exports for backward compatibility
pub use chain::get_last_log_hash;
pub use logger::{log_audit, log_audit_and_return};
pub use rotation::trim_log_file;
pub use types::{AuditEntry, AuditParams, AuditParamsBuilder, AuditStatus};
