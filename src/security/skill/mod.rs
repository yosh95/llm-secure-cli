//! Skill verification module — three-tier verification pipeline
//! for Agent Skills (SKILL.md) with structural validation,
//! cryptographic signature verification, and semantic firewall analysis.

pub mod parser;
pub mod semantic;
pub mod signature;
pub mod types;
pub mod verifier;

// Re-exports for backward compatibility
pub use parser::{discover_skills, parse_skill_file, validate_skill_structure};
pub use semantic::analyze_skill_semantic;
pub use signature::verify_skill_signature;
pub use types::{
    SkillContent, SkillMetadata, SkillSemanticFinding, SkillSemanticVerdict, SkillSignatureStatus,
    SkillStructureResult, SkillValidationError, SkillVerdict, SkillVerificationReport,
};
pub use verifier::verify_skill;
