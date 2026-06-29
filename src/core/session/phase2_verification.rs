use crate::core::session::ActiveSession;
use crate::security::verifier::{VerificationOutcome, VerificationParams};
use serde_json::Value;

impl ActiveSession {
    /// Build a consistent audit context for logging in Phase 2.
    fn build_audit_context(&self) -> serde_json::Value {
        let user_id = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());
        serde_json::json!({
            "trace_id": self.trace_id,
            "model": self.client.get_state().model.clone(),
            "provider": self.client.get_state().provider.clone(),
            "user_id": user_id
        })
    }

    /// Phase 2: Verification & Approval
    ///
    /// The verifier committee is evaluated **sequentially under a strict
    /// any-flag policy**: members are queried one at a time, in order, and the
    /// **first** member that flags the call (`NeedsApproval` or
    /// `FallbackRequired`) immediately hands off to human-in-the-loop approval —
    /// no remaining members are queried.  Only if **every** member approves is
    /// the call auto-approved.
    ///
    /// Returns:
    ///   - `(effective_args, auto_approved, cancel_msg)` where:
    ///     - `effective_args` are the tool arguments (unchanged, no MODIFY)
    ///     - `auto_approved` is `true` if the verifier greenlit the call
    ///     - `cancel_msg` is `Some(...)` with user feedback if rejected, or `None`
    pub(crate) fn phase2_verification(
        &mut self,
        name: &str,
        args: &serde_json::Map<String, Value>,
        config: &crate::config::models::AppConfig,
    ) -> anyhow::Result<(serde_json::Map<String, Value>, bool, Option<Value>)> {
        // In stdout/pipe mode: auto-approve all tool calls (no human interaction possible)
        if self.client.get_state().stdout {
            return Ok((args.clone(), true, None));
        }

        // Human-in-the-Loop guardrail.
        // When HITL is disabled (via --disable-human-in-the-loop CLI flag),
        // skip ALL verification and auto-approve every tool call.
        // This is the FINAL GUARDRAIL — disabling it bypasses all safety checks.
        if !self.hitl_enabled {
            self.ctx.ui.report_warning(
                "HITL DISABLED: Auto-approving tool call (--disable-human-in-the-loop was set at startup)"
            );
            let audit_ctx = self.build_audit_context();
            crate::security::audit::AuditParams::builder("verifier_decision", name, config)
                .args(serde_json::json!({
                    "verdict": "Allowed",
                    "reason": "HITL disabled via --disable-human-in-the-loop",
                    "auto_approved": true,
                }))
                .context(&audit_ctx)
                .log();
            return Ok((args.clone(), true, None));
        }

        // Display the tool call first — before any verifier queries.
        // This ensures the user sees the proposed action before verifier evaluation begins.
        self.ctx.ui.print_tool_call(name, &serde_json::json!(args));

        // 2a. Resolve Verifier Committee members
        let (committee_members, verifier_available) =
            self.ctx.config_manager.get_verifier_committee();

        if !verifier_available || committee_members.is_empty() {
            // Verifier is off or not configured: fall back to human approval.
            // Tool call already displayed above — no need to print again.

            // Audit log: verifier was not available, human will decide
            let audit_ctx = self.build_audit_context();
            crate::security::audit::AuditParams::builder("verifier_decision", name, config)
                .args(serde_json::json!({
                    "verdict": "NoVerifier",
                    "reason": "Verifier not configured or unavailable; falling back to manual approval",
                    "auto_approved": false,
                }))
                .context(&audit_ctx)
                .log();

            let cancel_msg = self.request_human_approval(name, config, "no_verifier")?;
            return if let Some(msg) = cancel_msg {
                Ok((args.clone(), false, Some(msg)))
            } else {
                Ok((args.clone(), false, None))
            };
        }

        // 2b. Sequential any-flag evaluation.
        let security_config = match self.ctx.config_manager.get_config() {
            Ok(c) => c.security.clone(),
            Err(_) => crate::config::models::SecurityConfig::default(),
        };
        let args_value = serde_json::json!(args);
        let member_count = committee_members.len();
        // Snapshot the cancellation counter so a Ctrl+C during any verifier
        // request aborts the whole turn (rather than being treated as a flag).
        let cancel_gen = self.cancel_token.generation();

        for (idx, (provider, model)) in committee_members.iter().enumerate() {
            eprintln!(
                "🧐 Verifier {}/{}: {provider}:{model}",
                idx + 1,
                member_count
            );

            let outcome = crate::security::verifier::verify_tool_call_full(VerificationParams {
                ctx_app: self.ctx.clone(),
                user_query: "",
                tool_name: name,
                tool_args: &args_value,
                context: None,
                config: &security_config,
                provider: Some(provider.clone()),
                model: Some(model.clone()),
            });

            // Ctrl+C during verification aborts the whole turn.
            if crate::core::session::cancelled_since(cancel_gen) {
                eprintln!("(Interrupted by Ctrl+C)");
                self.handle_interruption();
                return Err(anyhow::anyhow!("Interrupted during verification"));
            }

            match outcome {
                VerificationOutcome::Allowed(_reason) => {
                    // This member approved — keep checking the rest.
                }
                VerificationOutcome::NeedsApproval(reason) => {
                    let label = format!("{provider}:{model}");
                    return self.flag_to_human(
                        name,
                        args,
                        config,
                        &label,
                        &reason,
                        "verifier_needs_approval",
                        "NeedsApproval",
                    );
                }
                VerificationOutcome::FallbackRequired(reason) => {
                    let label = format!("{provider}:{model}");
                    return self.flag_to_human(
                        name,
                        args,
                        config,
                        &label,
                        &reason,
                        "verifier_fallback",
                        "FallbackRequired",
                    );
                }
            }
        }

        // 2c. Every member approved → auto-approve.
        let audit_ctx = self.build_audit_context();
        let reason = format!("All {member_count} committee member(s) approved.");
        crate::security::audit::AuditParams::builder("verifier_decision", name, config)
            .args(serde_json::json!({
                "verdict": "Allowed",
                "reason": reason,
                "auto_approved": true,
            }))
            .context(&audit_ctx)
            .log();

        self.ctx.ui.report_success(&format!(
            "Tool Call Approved (Auto-Approved): all {member_count} verifier(s) agreed."
        ));
        Ok((args.clone(), true, None))
    }

    /// A verifier flagged the tool call — audit the decision and ask the human
    /// to approve or reject (with optional feedback).
    ///
    /// NOTE: The tool call was already displayed before verifier evaluation began,
    /// so we do NOT re-print it here.
    #[allow(clippy::too_many_arguments)]
    fn flag_to_human(
        &mut self,
        name: &str,
        args: &serde_json::Map<String, Value>,
        config: &crate::config::models::AppConfig,
        member_label: &str,
        reason: &str,
        verifier_context: &str,
        verdict: &str,
    ) -> anyhow::Result<(serde_json::Map<String, Value>, bool, Option<Value>)> {
        let audit_ctx = self.build_audit_context();
        crate::security::audit::AuditParams::builder("verifier_decision", name, config)
            .args(serde_json::json!({
                "verdict": verdict,
                "reason": reason,
                "flagged_by": member_label,
                "auto_approved": false,
            }))
            .context(&audit_ctx)
            .log();

        // Tool call was already displayed above (before verifier evaluation).
        if verdict == "FallbackRequired" {
            self.ctx
                .ui
                .report_warning(&format!("Verifier unavailable ({member_label}): {reason}"));
        } else {
            eprintln!("🚩 Verifier {member_label} flagged this tool call as requiring review.");
            eprintln!("  👉 {reason}");
        }

        let cancel_msg = self.request_human_approval(name, config, verifier_context)?;
        if let Some(msg) = cancel_msg {
            Ok((args.clone(), false, Some(msg)))
        } else {
            Ok((args.clone(), false, None))
        }
    }

    /// Request human approval for a tool call.
    ///
    /// Returns `Ok(None)` if approved, `Ok(Some(cancel_message))` if rejected
    /// with optional feedback, or `Err` if interrupted.
    ///
    /// Logs the human approval/rejection decision to the audit trail
    /// with the given `verifier_context` explaining why HITL was triggered.
    fn request_human_approval(
        &mut self,
        name: &str,
        config: &crate::config::models::AppConfig,
        verifier_context: &str,
    ) -> anyhow::Result<Option<Value>> {
        let audit_ctx = self.build_audit_context();

        let emoji = match name {
            "execute_python" => "\u{1f40d} ",
            "brave_search" => "\u{1f310} ",
            _ => "\u{1f527} ",
        };
        match self.ctx.ui.ask_confirm(&format!("{emoji}Execute {name}")) {
            Some(crate::cli::ui::ConfirmResult::Yes) => {
                // Audit log: human approved the tool call
                crate::security::audit::AuditParams::builder("human_approval", name, config)
                    .args(serde_json::json!({
                        "result": "approved",
                        "verifier_context": verifier_context,
                    }))
                    .context(&audit_ctx)
                    .log();
                Ok(None)
            }
            Some(res) => {
                let feedback = match res {
                    crate::cli::ui::ConfirmResult::Feedback(f) => Some(f),
                    _ => None,
                };

                // Audit log: human rejected the tool call, with optional feedback
                let feedback_text = feedback.clone().unwrap_or_default();
                crate::security::audit::AuditParams::builder("human_approval", name, config)
                    .args(serde_json::json!({
                        "result": "rejected",
                        "verifier_context": verifier_context,
                        "feedback": feedback_text,
                    }))
                    .context(&audit_ctx)
                    .log();

                let cancel_msg = self.handle_rejection_feedback(feedback)?;
                Ok(Some(cancel_msg))
            }
            None => {
                self.handle_interruption();
                Err(anyhow::anyhow!("Interrupted"))
            }
        }
    }

    /// Handle feedback from user when they reject a tool call.
    /// Returns a Value that can be passed back to the LLM as a tool result.
    pub(crate) fn handle_rejection_feedback(
        &mut self,
        feedback: Option<String>,
    ) -> anyhow::Result<Value> {
        self.ctx.ui.report_warning("Execution cancelled by user.");
        let feedback = if let Some(f) = feedback {
            Some(f)
        } else {
            let f = crate::cli::ui::get_user_input("Provide feedback (optional): ");
            if let Some(ref content) = f
                && !content.trim().is_empty()
            {
                println!("  Feedback: {content}");
            }
            f
        };

        match feedback {
            Some(f) if !f.trim().is_empty() => Ok(Value::String(format!(
                "Error: Cancelled by user. Feedback: {f}"
            ))),
            Some(_) => Ok(Value::String("Error: Cancelled by user.".into())),
            None => {
                self.handle_interruption();
                Err(anyhow::anyhow!("Interrupted"))
            }
        }
    }
}
