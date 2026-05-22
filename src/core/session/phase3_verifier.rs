use crate::config::models::VerifierFallback;
use crate::core::session::ActiveSession;
use crate::security::dual_llm_verifier::VerificationOutcome;
use serde_json::Value;
use std::io::Write;

impl ActiveSession {
    /// Phase 3: Resolve Dual LLM verification outcome. Returns effective
    /// (possibly corrected) arguments for the tool call.
    pub(crate) async fn phase3_dual_llm_verification(
        &mut self,
        name: &str,
        args: &serde_json::Map<String, Value>,
        verifier_handle: Option<tokio::task::JoinHandle<VerificationOutcome>>,
    ) -> anyhow::Result<serde_json::Map<String, Value>> {
        let handle = match verifier_handle {
            Some(h) => h,
            None => return Ok(args.clone()),
        };

        let outcome = self.resolve_verifier_outcome(handle).await?;
        match outcome {
            VerificationOutcome::Allowed(reason) => {
                self.ctx
                    .ui
                    .report_success(&format!("Intent Verified: {}", reason));
                Ok(args.clone())
            }
            VerificationOutcome::Modified(fixed_args, reason) => {
                self.ctx
                    .ui
                    .report_success(&format!("Intent Verified & Corrected: {}", reason));
                if let Some(obj) = fixed_args.as_object() {
                    Ok(obj.clone())
                } else {
                    Ok(args.clone())
                }
            }
            VerificationOutcome::Rejected(reason) => {
                let msg = format!("Security Policy Violation: {}", reason);
                self.ctx.ui.report_error(&msg);
                Err(anyhow::anyhow!("Phase 3 rejected: {}", msg))
            }
            VerificationOutcome::FallbackRequired(reason) => {
                if !self.handle_verifier_fallback(name, &reason).await {
                    return Err(anyhow::anyhow!(
                        "Blocked (Verifier Unavailable): {}",
                        reason
                    ));
                }
                Ok(args.clone())
            }
        }
    }

    /// Resolves the result of the verifier task with interrupt support.
    async fn resolve_verifier_outcome(
        &mut self,
        handle: tokio::task::JoinHandle<VerificationOutcome>,
    ) -> anyhow::Result<VerificationOutcome> {
        print!("Finalizing intent verification... ");
        std::io::stdout().flush().ok();

        const VERIFIER_TIMEOUT_SECS: u64 = 60;

        let res = tokio::select! {
            res = handle => res.unwrap_or(VerificationOutcome::FallbackRequired("Task Panicked".into())),
            _ = tokio::time::sleep(std::time::Duration::from_secs(VERIFIER_TIMEOUT_SECS)) => {
                println!("\nVerifier timed out after {}s.", VERIFIER_TIMEOUT_SECS);
                VerificationOutcome::FallbackRequired(format!(
                    "Verifier timed out after {}s",
                    VERIFIER_TIMEOUT_SECS
                ))
            }
            _ = tokio::signal::ctrl_c() => {
                println!("\n^C - Interrupted.");
                self.handle_interruption();
                return Err(anyhow::anyhow!("Interrupted during verification"));
            }
        };
        println!("done");
        Ok(res)
    }

    async fn handle_verifier_fallback(&self, name: &str, reason: &str) -> bool {
        let config = match self.ctx.config_manager.get_config() {
            Ok(c) => c,
            Err(_) => return false,
        };
        match config.security.verifier_fallback {
            VerifierFallback::Block => {
                self.ctx
                    .ui
                    .report_error(&format!("Verifier unavailable — blocked: {}", reason));
                false
            }
            _ => {
                self.ctx
                    .ui
                    .report_warning(&format!("⚠ Verifier unavailable: {}", reason));
                matches!(
                    self.ctx
                        .ui
                        .ask_confirm(&format!("Execute {} (Manual confirmation required)", name))
                        .await,
                    Some(crate::cli::ui::ConfirmResult::Yes)
                )
            }
        }
    }
}
