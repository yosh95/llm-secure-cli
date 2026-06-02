use crate::security::skill::parser::validate_skill_structure;
use crate::security::skill::semantic::analyze_skill_semantic;
use crate::security::skill::signature::verify_skill_signature;
use crate::security::skill::types::{
    SkillSemanticVerdict, SkillSignatureStatus, SkillStructureResult, SkillVerdict,
    SkillVerificationReport,
};
use std::path::Path;

/// Runs the complete three-tier verification pipeline on a skill directory.
pub async fn verify_skill(
    dir: &Path,
    ctx: &std::sync::Arc<crate::core::context::AppContext>,
    config: &crate::config::models::SecurityConfig,
    run_semantic: bool,
    provider: Option<&str>,
    model: Option<&str>,
) -> SkillVerificationReport {
    let start = std::time::Instant::now();

    // Tier 1: Structural validation
    let structure = validate_skill_structure(dir);
    let skill_name = match &structure {
        SkillStructureResult::Pass { metadata } => metadata.name.clone(),
        SkillStructureResult::Fail { .. } => dir.file_name().map_or_else(
            || "unknown".to_string(),
            |n| n.to_string_lossy().to_string(),
        ),
    };

    // Tier 2: Signature verification
    let signature = if matches!(structure, SkillStructureResult::Pass { .. }) {
        verify_skill_signature(dir)
    } else {
        SkillSignatureStatus::Unsigned
    };

    // Tier 3: Semantic Firewall
    let semantic = if run_semantic && matches!(structure, SkillStructureResult::Pass { .. }) {
        analyze_skill_semantic(dir, ctx, config, provider, model).await
    } else {
        SkillSemanticVerdict::Skipped
    };

    // Determine overall verdict
    let verdict = compute_verdict(&structure, &signature, &semantic);

    SkillVerificationReport {
        skill_name,
        path: dir.to_string_lossy().to_string(),
        structure,
        signature,
        semantic: Some(semantic),
        verdict,
        total_duration_ms: start.elapsed().as_millis() as u64,
    }
}

/// Computes the overall verdict from the three-tier results.
fn compute_verdict(
    structure: &SkillStructureResult,
    signature: &SkillSignatureStatus,
    semantic: &SkillSemanticVerdict,
) -> SkillVerdict {
    // Structural failure → Dangerous
    if matches!(structure, SkillStructureResult::Fail { .. }) {
        return SkillVerdict::Dangerous;
    }

    // Signature verification failure → Suspicious
    if matches!(signature, SkillSignatureStatus::VerificationFailed(_)) {
        return SkillVerdict::Suspicious;
    }

    // Semantic verdict takes precedence
    match semantic {
        SkillSemanticVerdict::Toxic { .. } => SkillVerdict::Dangerous,
        SkillSemanticVerdict::Suspicious { .. } => SkillVerdict::Suspicious,
        SkillSemanticVerdict::Clean { .. } => {
            // Clean semantically, but unsigned → still Suspicious
            if matches!(signature, SkillSignatureStatus::Unsigned) {
                SkillVerdict::Suspicious
            } else {
                SkillVerdict::Safe
            }
        }
        SkillSemanticVerdict::Skipped => {
            // No semantic analysis available → check signature
            match signature {
                SkillSignatureStatus::Verified { .. } => SkillVerdict::Safe,
                SkillSignatureStatus::Unsigned => SkillVerdict::Suspicious,
                SkillSignatureStatus::VerificationFailed(_) => SkillVerdict::Suspicious,
            }
        }
        SkillSemanticVerdict::Error { .. } => {
            // Semantic analysis failed → treat as Suspicious
            SkillVerdict::Suspicious
        }
    }
}
