use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Skill Metadata (parsed from SKILL.md YAML frontmatter) ─────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMetadata {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<Vec<String>>,
}

// ── Validation errors ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum SkillValidationError {
    MissingSkillMd,
    NoYamlFrontmatter,
    InvalidYaml(String),
    MissingName,
    MissingDescription,
    NameTooLong { actual: usize, max: usize },
    DescriptionTooLong { actual: usize, max: usize },
    InvalidName(String),
}

impl std::fmt::Display for SkillValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkillValidationError::MissingSkillMd => {
                write!(f, "SKILL.md file not found in directory")
            }
            SkillValidationError::NoYamlFrontmatter => {
                write!(f, "No YAML frontmatter (--- ... ---) found in SKILL.md")
            }
            SkillValidationError::InvalidYaml(msg) => {
                write!(f, "Invalid YAML frontmatter: {msg}")
            }
            SkillValidationError::MissingName => {
                write!(f, "Required field 'name' is missing in frontmatter")
            }
            SkillValidationError::MissingDescription => {
                write!(f, "Required field 'description' is missing in frontmatter")
            }
            SkillValidationError::NameTooLong { actual, max } => {
                write!(
                    f,
                    "name is {actual} characters (max: {max}). Use lowercase letters, numbers, and hyphens."
                )
            }
            SkillValidationError::DescriptionTooLong { actual, max } => {
                write!(f, "description is {actual} characters (max: {max})")
            }
            SkillValidationError::InvalidName(name) => {
                write!(
                    f,
                    "name '{name}' contains invalid characters. Use only lowercase letters, numbers, and hyphens."
                )
            }
        }
    }
}

// ── Signature status ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub enum SkillSignatureStatus {
    Verified {
        publisher: String,
        algorithm: String,
    },
    Unsigned,
    VerificationFailed(String),
}

// ── Semantic Firewall verdict ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSemanticFinding {
    pub category: String,
    pub description: String,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize)]
pub enum SkillSemanticVerdict {
    Clean {
        confidence: f64,
    },
    Suspicious {
        findings: Vec<SkillSemanticFinding>,
        confidence: f64,
    },
    Toxic {
        findings: Vec<SkillSemanticFinding>,
        confidence: f64,
    },
    /// Semantic analysis was not requested (--semantic flag not passed).
    Skipped,
    /// Semantic analysis was requested but could not complete.
    /// The message explains why (e.g., provider not configured, API key missing, network error).
    Error {
        message: String,
    },
}

impl SkillSemanticVerdict {
    #[must_use]
    pub fn short_label(&self) -> &str {
        match self {
            SkillSemanticVerdict::Clean { .. } => "CLEAN",
            SkillSemanticVerdict::Suspicious { .. } => "SUSPICIOUS",
            SkillSemanticVerdict::Toxic { .. } => "TOXIC",
            SkillSemanticVerdict::Skipped => "SKIPPED",
            SkillSemanticVerdict::Error { .. } => "ERROR",
        }
    }

    #[must_use]
    pub fn confidence(&self) -> f64 {
        match self {
            SkillSemanticVerdict::Clean { confidence }
            | SkillSemanticVerdict::Suspicious { confidence, .. }
            | SkillSemanticVerdict::Toxic { confidence, .. } => *confidence,
            SkillSemanticVerdict::Skipped | SkillSemanticVerdict::Error { .. } => 0.0,
        }
    }
}

// ── Overall verdict ──────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum SkillVerdict {
    Safe,
    Suspicious,
    Dangerous,
}

// ── Full verification report ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct SkillVerificationReport {
    pub skill_name: String,
    pub path: String,
    pub structure: SkillStructureResult,
    pub signature: SkillSignatureStatus,
    pub semantic: Option<SkillSemanticVerdict>,
    pub verdict: SkillVerdict,
    pub total_duration_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status")]
pub enum SkillStructureResult {
    Pass { metadata: SkillMetadata },
    Fail { errors: Vec<String> },
}

// ── Raw skill content (for semantic analysis) ────────────────────────────

#[derive(Debug, Clone)]
pub struct SkillContent {
    pub metadata: SkillMetadata,
    pub body: String,
    pub raw_frontmatter: String,
}

// ── Constants per the Agent Skills spec ──────────────────────────────────

pub const MAX_NAME_LENGTH: usize = 64;
pub const MAX_DESCRIPTION_LENGTH: usize = 1024;
pub const VALID_NAME_PATTERN: &str = "abcdefghijklmnopqrstuvwxyz0123456789-";
