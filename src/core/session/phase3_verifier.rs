use crate::core::session::ActiveSession;
use crate::security::dual_llm_verifier::VerificationOutcome;
use serde_json::Value;
use std::io::Write;

impl ActiveSession {
    /// Phase 3: Resolve Dual LLM verification outcome.
    ///
    /// Returns:
    ///   - (effective_args, auto_approved) where auto_approved=true means the
    ///     verifier approved and execution proceeds without human intervention.
    pub(crate) async fn phase3_dual_llm_verification(
        &mut self,
        name: &str,
        args: &serde_json::Map<String, Value>,
        verifier_handle: Option<tokio::task::JoinHandle<VerificationOutcome>>,
    ) -> anyhow::Result<(serde_json::Map<String, Value>, bool)> {
        let handle = match verifier_handle {
            Some(h) => h,
            None => {
                // No verifier was spawned (verifier off/unconfigured).
                // Human approval was already handled in Phase 2.
                return Ok((args.clone(), false));
            }
        };

        let outcome = self.resolve_verifier_outcome(handle).await?;
        match outcome {
            VerificationOutcome::Allowed(reason) => {
                // Verifier says it's safe → auto-approve.
                self.ctx
                    .ui
                    .print_tool_call_direct(name, &serde_json::json!(args));
                self.ctx
                    .ui
                    .report_success(&format!("Intent Verified (Auto-Approved): {}", reason));
                Ok((args.clone(), true))
            }
            VerificationOutcome::Modified(fixed_args, reason) => {
                // Verifier says it's safe with corrections → auto-approve with corrected args.
                self.ctx
                    .ui
                    .print_tool_call_direct(name, &serde_json::json!(args));
                self.ctx.ui.report_success(&format!(
                    "Intent Verified & Corrected (Auto-Approved): {}",
                    reason
                ));
                let effective_args = if let Some(obj) = fixed_args.as_object() {
                    obj.clone()
                } else {
                    args.clone()
                };
                Ok((effective_args, true))
            }
            VerificationOutcome::NeedsApproval(reason) => {
                // Verifier flagged as potentially unsafe.
                self.ctx.ui.print_tool_call(name, &serde_json::json!(args));
                self.ctx
                    .ui
                    .report_warning("Verifier flagged this tool call as requiring review.");
                self.ctx.ui.report_info(&format!("Reason: {}", reason));
                if self.ask_human_for_approval(name).await {
                    Ok((args.clone(), false))
                } else {
                    Err(anyhow::anyhow!("Rejected by user after verifier review"))
                }
            }
            VerificationOutcome::FallbackRequired(reason) => {
                // Verifier unavailable — always ask for human approval.
                self.ctx.ui.print_tool_call(name, &serde_json::json!(args));
                if !self.handle_verifier_fallback(name, &reason).await {
                    return Err(anyhow::anyhow!(
                        "Blocked (Verifier Unavailable): {}",
                        reason
                    ));
                }
                Ok((args.clone(), false))
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
                println!("
        Verifier timed out after {}s.", VERIFIER_TIMEOUT_SECS);
                VerificationOutcome::FallbackRequired(format!(
                    "Verifier timed out after {}s",
                    VERIFIER_TIMEOUT_SECS
                ))
            }
            _ = tokio::signal::ctrl_c() => {
                println!("
        ^C - Interrupted.");
                self.handle_interruption();
                return Err(anyhow::anyhow!("Interrupted during verification"));
            }
        };
        println!("done");
        Ok(res)
    }

    /// Ask the human for approval when the verifier flagged the call.
    async fn ask_human_for_approval(&self, name: &str) -> bool {
        matches!(
            self.ctx
                .ui
                .ask_confirm(&format!(
                    "Execute {} (Manual approval required — see reason above)",
                    name
                ))
                .await,
            Some(crate::cli::ui::ConfirmResult::Yes)
        )
    }

    /// Handle verifier fallback (verifier unavailable).
    /// Always asks for human approval — the "block" option has been removed.
    async fn handle_verifier_fallback(&self, name: &str, reason: &str) -> bool {
        self.ctx
            .ui
            .report_warning(&format!("Verifier unavailable: {}", reason));
        matches!(
            self.ctx
                .ui
                .ask_confirm(&format!("Execute {} (Manual confirmation required)", name))
                .await,
            Some(crate::cli::ui::ConfirmResult::Yes)
        )
    }
}
