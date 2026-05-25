use crate::core::session::ActiveSession;
use crate::security::dual_llm_verifier::{
    CommitteeVerdict, VerificationOutcome, VerificationParams,
};
use serde_json::Value;
use std::io::Write;

impl ActiveSession {
    /// Phase 2: Verification & Approval
    ///
    /// Combines Zero Trust enforcement, verifier committee resolution & spawning,
    /// and verifier outcome resolution into a single phase.  The Verifier LLM
    /// handles all risk judgements — the old CASS-based approach has been removed.
    ///
    /// Returns:
    ///   - `(effective_args, auto_approved, cancel_msg)` where:
    ///     - `effective_args` are the (possibly corrected) tool arguments
    ///     - `auto_approved` is `true` if the verifier greenlit the call
    ///     - `cancel_msg` is `Some(...)` with user feedback if rejected, or `None`
    pub(crate) async fn phase2_verification(
        &mut self,
        name: &str,
        args: &serde_json::Map<String, Value>,
        config: &crate::config::models::AppConfig,
    ) -> anyhow::Result<(serde_json::Map<String, Value>, bool, Option<Value>)> {
        // 2a. Zero Trust enforcement for MCP servers
        if self.check_zero_trust(name, &config.mcp_servers) {
            match crate::security::identity::IdentityManager::generate_token(Some(name)) {
                Ok(_token) => self
                    .ctx
                    .ui
                    .report_success("Identity Verified (Hybrid PQC Token generated)"),
                Err(e) => self
                    .ctx
                    .ui
                    .report_warning(&format!("Identity Verification failed: {}", e)),
            }
        }

        // 2b. Resolve Verifier Committee members
        let (committee_members, verifier_available) =
            self.ctx.config_manager.get_verifier_committee();

        if verifier_available && !committee_members.is_empty() {
            // Spawn verifier task(s) and resolve outcome
            let verifier_handle = if committee_members.len() == 1 {
                // Single verifier (legacy mode)
                let (v_provider, v_model) = &committee_members[0];
                self.ctx.ui.report_info(&format!(
                    "Verifier: {} (single-member committee)",
                    committee_members[0].1,
                ));
                Some(self.spawn_verifier_task(name, args, v_provider.clone(), v_model.clone()))
            } else {
                // Multi-member committee — any-flag policy
                let member_count = committee_members.len();
                self.ctx.ui.report_info(&format!(
                    "Verifier Committee: {} members (any-flag policy)",
                    member_count,
                ));
                Some(self.spawn_committee_task(name, args, committee_members))
            };

            self.resolve_verifier_outcome(name, args, verifier_handle)
                .await
        } else {
            // Verifier is off or not configured: fall back to human approval.
            if config.security.dual_llm_verification.unwrap_or(false) {
                self.ctx.ui.report_warning(
                    "Dual LLM verification is enabled, but no verifier committee members are configured.                      Falling back to manual approval.",
                );
                self.ctx
                    .ui
                    .report_info("Hint: Use /vp and /vm to set the primary verifier, or add [security.verifier_committee] members.");
            }
            // Show the tool call — human needs to review
            self.ctx.ui.print_tool_call(name, &serde_json::json!(args));
            // No verifier task — ask human directly
            let cancel_msg = self.request_human_approval(name).await?;
            if let Some(msg) = cancel_msg {
                Ok((args.clone(), false, Some(msg)))
            } else {
                Ok((args.clone(), false, None))
            }
        }
    }

    /// Resolve the verifier outcome: wait for the spawned task, interpret the result,
    /// and either auto-approve or ask the human for approval.
    async fn resolve_verifier_outcome(
        &mut self,
        name: &str,
        args: &serde_json::Map<String, Value>,
        verifier_handle: Option<tokio::task::JoinHandle<VerificationOutcome>>,
    ) -> anyhow::Result<(serde_json::Map<String, Value>, bool, Option<Value>)> {
        let handle = match verifier_handle {
            Some(h) => h,
            None => {
                // Shouldn't happen, but handle gracefully
                return Ok((args.clone(), false, None));
            }
        };

        let outcome = self.wait_for_verifier(handle).await?;
        match outcome {
            VerificationOutcome::Allowed(reason) => {
                // Verifier says it's safe → auto-approve.
                self.ctx
                    .ui
                    .print_tool_call_direct(name, &serde_json::json!(args));
                self.ctx
                    .ui
                    .report_success(&format!("Intent Verified (Auto-Approved): {}", reason));
                Ok((args.clone(), true, None))
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
                Ok((effective_args, true, None))
            }
            VerificationOutcome::NeedsApproval(reason) => {
                // Verifier flagged as potentially unsafe.
                // Ask human for approval with feedback support.
                // The verifier task has already completed, so no abort is needed.
                self.ctx.ui.print_tool_call(name, &serde_json::json!(args));
                self.ctx
                    .ui
                    .report_warning("Verifier flagged this tool call as requiring review.");
                self.ctx.ui.report_info(&format!("Reason: {}", reason));
                let cancel_msg = self.request_human_approval(name).await?;
                if let Some(msg) = cancel_msg {
                    // User rejected with feedback — return it as cancel message
                    Ok((args.clone(), false, Some(msg)))
                } else {
                    // User approved
                    Ok((args.clone(), false, None))
                }
            }
            VerificationOutcome::FallbackRequired(reason) => {
                // Verifier unavailable — ask for human approval with feedback support.
                // The verifier task has already completed (or timed out), so no abort is needed.
                self.ctx.ui.print_tool_call(name, &serde_json::json!(args));
                self.ctx
                    .ui
                    .report_warning(&format!("Verifier unavailable: {}", reason));
                let cancel_msg = self.request_human_approval(name).await?;
                if let Some(msg) = cancel_msg {
                    Ok((args.clone(), false, Some(msg)))
                } else {
                    Ok((args.clone(), false, None))
                }
            }
        }
    }

    /// Wait for the verifier task to complete, with timeout and interrupt support.
    async fn wait_for_verifier(
        &mut self,
        handle: tokio::task::JoinHandle<VerificationOutcome>,
    ) -> anyhow::Result<VerificationOutcome> {
        print!("Finalizing intent verification... ");
        std::io::stdout().flush().ok();

        const VERIFIER_TIMEOUT_SECS: u64 = 60;

        let res = tokio::select! {
            res = handle => res.unwrap_or(VerificationOutcome::FallbackRequired("Task Panicked".into())),
            _ = tokio::time::sleep(std::time::Duration::from_secs(VERIFIER_TIMEOUT_SECS)) => {
                println!("\n\tVerifier timed out after {}s.", VERIFIER_TIMEOUT_SECS);
                VerificationOutcome::FallbackRequired(format!(
                    "Verifier timed out after {}s",
                    VERIFIER_TIMEOUT_SECS
                ))
            }
            _ = tokio::signal::ctrl_c() => {
                println!("\n\t^C - Interrupted.");
                self.handle_interruption();
                return Err(anyhow::anyhow!("Interrupted during verification"));
            }
        };
        println!("done");
        Ok(res)
    }

    /// Check whether a Zero Trust MCP policy applies for this tool.
    fn check_zero_trust(
        &self,
        name: &str,
        mcp_servers: &[crate::config::models::McpServerConfig],
    ) -> bool {
        if !name.contains("__") {
            return false;
        }
        let server_name = name.split("__").next().unwrap_or_default();
        if let Some(mcp_config) = mcp_servers.iter().find(|s| s.name == server_name)
            && mcp_config.zero_trust
        {
            self.ctx.ui.report_info(&format!(
                "Zero Trust Policy enabled for server '{}'.",
                server_name
            ));
            true
        } else {
            false
        }
    }

    /// Request human approval for a tool call.
    ///
    /// Returns `Ok(None)` if approved, `Ok(Some(cancel_message))` if rejected
    /// with optional feedback, or `Err` if interrupted.
    async fn request_human_approval(&mut self, name: &str) -> anyhow::Result<Option<Value>> {
        match self.ctx.ui.ask_confirm(&format!("Execute {}", name)).await {
            Some(crate::cli::ui::ConfirmResult::Yes) => Ok(None),
            Some(res) => {
                let feedback = match res {
                    crate::cli::ui::ConfirmResult::Feedback(f) => Some(f),
                    _ => None,
                };
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
        let feedback = match feedback {
            Some(f) => Some(f),
            None => {
                let f = crate::cli::ui::get_user_input("Provide feedback (optional): ");
                if let Some(ref content) = f
                    && !content.trim().is_empty()
                {
                    use colored::*;
                    println!("  {}", format!("Feedback: {}", content).dimmed());
                }
                f
            }
        };

        match feedback {
            Some(f) if !f.trim().is_empty() => Ok(Value::String(format!(
                "Error: Cancelled by user. Feedback: {}",
                f
            ))),
            Some(_) => Ok(Value::String("Error: Cancelled by user.".into())),
            None => {
                self.handle_interruption();
                Err(anyhow::anyhow!("Interrupted"))
            }
        }
    }

    /// Spawns the Dual LLM Verification task.
    fn spawn_verifier_task(
        &self,
        name: &str,
        args: &serde_json::Map<String, Value>,
        provider: String,
        model: String,
    ) -> tokio::task::JoinHandle<VerificationOutcome> {
        let ctx_clone = self.ctx.clone();
        let config_clone = match self.ctx.config_manager.get_config() {
            Ok(c) => c.security.clone(),
            Err(_) => crate::config::models::SecurityConfig::default(),
        };
        let intent_context = self.get_intent_context();
        let name_clone = name.to_string();
        let args_clone = serde_json::json!(args);

        tokio::spawn(async move {
            crate::security::dual_llm_verifier::verify_tool_call_full(VerificationParams {
                ctx_app: ctx_clone,
                user_query: &intent_context,
                tool_name: &name_clone,
                tool_args: &args_clone,
                context: None,
                config: &config_clone,
                provider: Some(provider),
                model: Some(model),
            })
            .await
        })
    }

    /// Spawns the aggregated committee verification task.
    ///
    /// Runs ALL committee members concurrently and aggregates their verdicts
    /// using the "any-flag" policy:
    /// - If ANY member flags NeedsApproval → the aggregated result is NeedsApproval.
    /// - Only if ALL members return Allowed → the result is Allowed.
    /// - If ANY member is unavailable → FallbackRequired.
    fn spawn_committee_task(
        &self,
        name: &str,
        args: &serde_json::Map<String, Value>,
        committee_members: Vec<(String, String)>,
    ) -> tokio::task::JoinHandle<VerificationOutcome> {
        let ctx_clone = self.ctx.clone();
        let config_clone = match self.ctx.config_manager.get_config() {
            Ok(c) => c.security.clone(),
            Err(_) => crate::config::models::SecurityConfig::default(),
        };
        let intent_context = self.get_intent_context();
        let name_clone = name.to_string();
        let args_clone = serde_json::json!(args);

        tokio::spawn(async move {
            let committee_verdict = crate::security::dual_llm_verifier::verify_committee(
                VerificationParams {
                    ctx_app: ctx_clone,
                    user_query: &intent_context,
                    tool_name: &name_clone,
                    tool_args: &args_clone,
                    context: None,
                    config: &config_clone,
                    provider: None,
                    model: None,
                },
                &committee_members,
            )
            .await;

            // Convert CommitteeVerdict → VerificationOutcome for backward compatibility
            match committee_verdict {
                CommitteeVerdict::Allowed => {
                    VerificationOutcome::Allowed("All committee members approved.".to_string())
                }
                CommitteeVerdict::Modified(fixed_args, reason) => {
                    VerificationOutcome::Modified(fixed_args, reason)
                }
                CommitteeVerdict::NeedsApproval(details) => {
                    let summary: Vec<String> = details
                        .iter()
                        .map(|(provider, model, reason)| {
                            format!("[{}@{}] {}", provider, model, reason)
                        })
                        .collect();
                    VerificationOutcome::NeedsApproval(format!(
                        "Committee flagged by {} member(s): {}",
                        details.len(),
                        summary.join(" | ")
                    ))
                }
                CommitteeVerdict::FallbackRequired(details) => {
                    let summary: Vec<String> = details
                        .iter()
                        .map(|(provider, model, reason)| {
                            format!("[{}@{}] {}", provider, model, reason)
                        })
                        .collect();
                    VerificationOutcome::FallbackRequired(format!(
                        "Committee fallback for {} member(s): {}",
                        details.len(),
                        summary.join(" | ")
                    ))
                }
            }
        })
    }

    /// Extract the user's intent context from recent conversation history.
    pub(crate) fn get_intent_context(&self) -> String {
        use crate::llm::models::Role;

        const MAX_MSG_CHARS: usize = 1000;
        const MAX_TOTAL_CHARS: usize = 4000;
        const HEAD_TAIL_CHARS: usize = 500;

        let history: Vec<String> = self
            .client
            .get_state()
            .conversation
            .iter()
            .filter(|m| m.role == Role::User)
            .rev()
            .take(5)
            .map(|m| {
                let text = m.get_text(true);
                let len = text.chars().count();
                if len > MAX_MSG_CHARS {
                    let chars: Vec<char> = text.chars().collect();
                    let head: String = chars.iter().take(HEAD_TAIL_CHARS).collect();
                    let tail: String = chars
                        .iter()
                        .rev()
                        .take(HEAD_TAIL_CHARS)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();
                    format!("{}...[TRUNCATED]...{}", head, tail)
                } else {
                    text
                }
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>();

        let context = history.join("\n---\n");
        match context.char_indices().nth(MAX_TOTAL_CHARS) {
            Some((cut_at, _)) => context[..cut_at].to_string(),
            None => context,
        }
    }
}
