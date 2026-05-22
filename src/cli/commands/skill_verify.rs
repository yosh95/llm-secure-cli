use crate::cli::ui;
use crate::security::skill_verifier::{
    SkillSemanticVerdict, SkillSignatureStatus, SkillStructureResult, SkillVerdict,
    discover_skills, verify_skill,
};
use std::path::Path;

/// Runs a full verification report on a skill directory, printing
/// a human-readable report to stdout.
pub async fn run_skill_verify(
    ctx: &std::sync::Arc<crate::core::context::AppContext>,
    path: &str,
    recursive: bool,
    run_semantic: bool,
    output_json: bool,
    provider: Option<&str>,
    model: Option<&str>,
) {
    let config = match ctx.config_manager.get_config() {
        Ok(c) => c,
        Err(e) => {
            ui::report_error(&format!("Cannot load config: {}", e));
            return;
        }
    };

    let root = Path::new(path);

    if !root.exists() {
        ui::report_error(&format!("Path not found: {}", path));
        return;
    }

    if recursive && root.is_dir() && !has_direct_skill_md(root) {
        // Batch mode: discover all skills under root
        let skill_dirs = discover_skills(root, true);
        if skill_dirs.is_empty() {
            ui::report_warning(&format!(
                "No skill directories (containing SKILL.md) found under {}",
                path
            ));
            return;
        }
        println!("Found {} skill(s) under {}\n", skill_dirs.len(), path);

        let mut reports = Vec::new();
        let total = skill_dirs.len();
        for (i, dir) in skill_dirs.iter().enumerate() {
            let report =
                verify_skill(dir, ctx, &config.security, run_semantic, provider, model).await;

            if !output_json {
                println!(
                    "[{}/{}] {}",
                    i + 1,
                    total,
                    dir.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "?".to_string())
                );
                print_report_summary(&report);
                println!();
            }

            reports.push(report);
        }

        if output_json {
            println!(
                "{}",
                serde_json::to_string_pretty(&reports).unwrap_or_else(|_| "[]".to_string())
            );
        } else {
            // Print summary
            let safe = reports
                .iter()
                .filter(|r| r.verdict == SkillVerdict::Safe)
                .count();
            let suspicious = reports
                .iter()
                .filter(|r| r.verdict == SkillVerdict::Suspicious)
                .count();
            let dangerous = reports
                .iter()
                .filter(|r| r.verdict == SkillVerdict::Dangerous)
                .count();
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            println!(" Batch Summary: {} skills", total);
            println!(
                "   {} Safe | {} Suspicious | {} Dangerous",
                safe, suspicious, dangerous
            );
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        }
        return;
    }

    // Single skill verification
    if !has_direct_skill_md(root) {
        ui::report_error(&format!(
            "No SKILL.md found in {}. Use --recursive to scan subdirectories.",
            path
        ));
        return;
    }

    let report = verify_skill(root, ctx, &config.security, run_semantic, provider, model).await;

    if output_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "JSON serialization failed");
                String::new()
            })
        );
    } else {
        print_full_report(&report);
    }
}

fn has_direct_skill_md(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    let candidates = [
        path.join("SKILL.md"),
        path.join("skill.md"),
        path.join("Skill.md"),
    ];
    candidates.iter().any(|p| p.exists())
}

// ── Report formatting ─────────────────────────────────────────────────────

fn print_full_report(report: &crate::security::skill_verifier::SkillVerificationReport) {
    println!();
    println!("━━━ Skill Verification Report ━━━");
    println!("Skill: {}", report.skill_name);
    println!("Path:  {}", report.path);
    println!();

    // Tier 1: Structure
    print!("[1] Structure ....................................... ");
    match &report.structure {
        SkillStructureResult::Pass { metadata } => {
            println!("{}", style_green("✓ PASS"));
            println!("    SKILL.md: valid YAML frontmatter");
            println!("    name: {:?} (valid)", metadata.name);
            println!(
                "    description: {} chars (limit: 1024)",
                metadata.description.len()
            );
            if let Some(ref license) = metadata.license {
                println!("    license: {}", license);
            }
            if let Some(ref compat) = metadata.compatibility {
                println!("    compatibility: {}", compat);
            }
            if let Some(ref tools) = metadata.allowed_tools {
                println!("    allowed-tools: {:?}", tools);
            }
        }
        SkillStructureResult::Fail { errors } => {
            println!("{}", style_red("✗ FAIL"));
            for e in errors {
                println!("    ✗ {}", e);
            }
        }
    }
    println!();

    // Tier 2: Signature
    print!("[2] Signature ....................................... ");
    match &report.signature {
        SkillSignatureStatus::Verified {
            publisher,
            algorithm,
        } => {
            println!("{}", style_green("✓ VERIFIED"));
            println!("    Publisher: {}", publisher);
            println!("    Algorithm: {}", algorithm);
        }
        SkillSignatureStatus::Unsigned => {
            println!("{}", style_yellow("△ UNSIGNED"));
        }
        SkillSignatureStatus::VerificationFailed(reason) => {
            println!("{}", style_red("✗ FAILED"));
            println!("    {}", reason);
        }
    }
    println!();

    // Tier 3: Semantic Firewall
    print!("[3] Semantic Firewall ............................... ");
    match &report.semantic {
        Some(SkillSemanticVerdict::Clean { confidence }) => {
            println!("{}", style_green("✓ CLEAN"));
            println!("    Confidence: {:.2}", confidence);
        }
        Some(SkillSemanticVerdict::Suspicious {
            findings,
            confidence,
        }) => {
            println!("{}", style_yellow("△ SUSPICIOUS"));
            println!("    Confidence: {:.2}", confidence);
            for finding in findings {
                println!(
                    "    ⚠ [{}] {} (confidence: {:.2})",
                    finding.category, finding.description, finding.confidence
                );
            }
        }
        Some(SkillSemanticVerdict::Toxic {
            findings,
            confidence,
        }) => {
            println!("{}", style_red("✗ TOXIC"));
            println!("    Confidence: {:.2}", confidence);
            for finding in findings {
                println!(
                    "    ⚠ [{}] {} (confidence: {:.2})",
                    finding.category, finding.description, finding.confidence
                );
            }
        }
        Some(SkillSemanticVerdict::Skipped) => {
            println!("{}", style_dim("— SKIPPED"));
            println!("    (Use --semantic to enable LLM-based analysis)");
        }
        Some(SkillSemanticVerdict::Error { message }) => {
            println!("{}", style_red("✗ ERROR"));
            println!("    {}", message);
        }
        None => {
            println!("{}", style_dim("— SKIPPED"));
            println!("    (Use --semantic to enable LLM-based analysis)");
        }
    }
    println!();

    // Verdict
    println!(
        "━━━ VERDICT: {} ━━━",
        match report.verdict {
            SkillVerdict::Safe => style_green_bold("SAFE"),
            SkillVerdict::Suspicious => style_yellow_bold("SUSPICIOUS"),
            SkillVerdict::Dangerous => style_red_bold("DANGEROUS"),
        }
    );

    match report.verdict {
        SkillVerdict::Safe => {
            if matches!(report.signature, SkillSignatureStatus::Unsigned) {
                println!("  ▶ Unsigned skill from unknown publisher");
            }
            println!("  ▶ No structural or semantic issues detected");
        }
        SkillVerdict::Suspicious => {
            println!("  ▶ Review the findings above before installing");
            if matches!(report.signature, SkillSignatureStatus::Unsigned) {
                println!("  ▶ Consider requesting the publisher to sign this skill");
            }
            if report.semantic.is_none() {
                println!("  ▶ Run with --semantic for deeper analysis");
            }
        }
        SkillVerdict::Dangerous => {
            println!("  ▶ DO NOT INSTALL");
            if matches!(report.structure, SkillStructureResult::Fail { .. }) {
                println!("  ▶ This skill does not conform to the Agent Skills specification");
            }
            if let Some(SkillSemanticVerdict::Toxic { .. }) = &report.semantic {
                println!("  ▶ This skill contains suspicious or malicious content");
            }
        }
    }
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    println!("\nTotal verification time: {}ms", report.total_duration_ms);
}

fn print_report_summary(report: &crate::security::skill_verifier::SkillVerificationReport) {
    let verdict_icon = match report.verdict {
        SkillVerdict::Safe => style_green("✓"),
        SkillVerdict::Suspicious => style_yellow("△"),
        SkillVerdict::Dangerous => style_red("✗"),
    };

    let struct_icon = match &report.structure {
        SkillStructureResult::Pass { .. } => style_green("✓"),
        SkillStructureResult::Fail { .. } => style_red("✗"),
    };

    let sig_icon = match &report.signature {
        SkillSignatureStatus::Verified { .. } => style_green("✓"),
        SkillSignatureStatus::Unsigned => style_dim("—"),
        SkillSignatureStatus::VerificationFailed(_) => style_red("✗"),
    };

    let sem_icon = match &report.semantic {
        Some(SkillSemanticVerdict::Clean { .. }) => style_green("✓"),
        Some(SkillSemanticVerdict::Suspicious { .. }) => style_yellow("△"),
        Some(SkillSemanticVerdict::Toxic { .. }) => style_red("✗"),
        Some(SkillSemanticVerdict::Skipped) => style_dim("—"),
        Some(SkillSemanticVerdict::Error { .. }) => style_red("✗"),
        None => style_dim("—"),
    };

    println!(
        "  Verdict: {} | Struct: {} | Sig: {} | Sem: {}",
        verdict_icon, struct_icon, sig_icon, sem_icon
    );
}

// ── Styling helpers ──────────────────────────────────────────────────────

fn style_green(text: &str) -> String {
    format!("\x1b[32m{}\x1b[0m", text)
}
fn style_green_bold(text: &str) -> String {
    format!("\x1b[1;32m{}\x1b[0m", text)
}
fn style_red(text: &str) -> String {
    format!("\x1b[31m{}\x1b[0m", text)
}
fn style_red_bold(text: &str) -> String {
    format!("\x1b[1;31m{}\x1b[0m", text)
}
fn style_yellow(text: &str) -> String {
    format!("\x1b[33m{}\x1b[0m", text)
}
fn style_yellow_bold(text: &str) -> String {
    format!("\x1b[1;33m{}\x1b[0m", text)
}
fn style_dim(text: &str) -> String {
    format!("\x1b[2m{}\x1b[0m", text)
}
