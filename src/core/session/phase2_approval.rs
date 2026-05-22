use crate::config::models::AutoApprovalLevel;
use crate::core::session::ActiveSession;
use crate::security::cass::RiskLevel;
use crate::security::dual_llm_verifier::{VerificationOutcome, VerificationParams};
use serde_json::Value;

impl ActiveSession {
    /// Phase 2: CASS risk evaluation, Zero Trust check, auto-approval determination,
    /// and human-in-the-loop approval. Returns the risk level, whether the tool is
    /// auto-approved, an optional verifier task handle, and an optional cancellation
    /// message (when the user rejects with feedback).
    pub(crate) async fn phase2_risk_and_approval(
        &mut self,
        name: &str,
        args: &serde_json::Map<String, Value>,
        config: &crate::config::models::AppConfig,
    ) -> anyhow::Result<(
        RiskLevel,
        bool,
        Option<tokio::task::JoinHandle<VerificationOutcome>>,
        Option<Value>,
    )> {
        // 2a. CASS risk evaluation
        let risk_level = crate::security::cass::CASS_ORCHESTRATOR.evaluate_risk(
            name,
            Some(&serde_json::json!(args)),
            &config.security,
        );

        // 2b. Zero Trust enforcement for MCP servers
        let force_manual = self.check_zero_trust(name, &config.mcp_servers);

        // 2c. Determine auto-approval status
        let approved = if force_manual {
            false
        } else {
            self.is_auto_approved(name, risk_level)
        };

        self.ctx.ui.print_tool_call(name, &serde_json::json!(args));

        // 2d. Generate identity token for Zero Trust MCP calls
        if force_manual {
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

        // 2e. Spawn Dual LLM verifier background task if applicable
        let verifier_handle = if !approved {
            self.maybe_spawn_verifier(name, args, &config.security)
        } else {
            None
        };

        // 2f. Request human approval if not auto-approved
        let cancel_msg = if !approved {
            self.request_human_approval(name, verifier_handle.as_ref())
                .await?
        } else {
            None
        };

        Ok((risk_level, approved, verifier_handle, cancel_msg))
    }

    /// Check whether a Zero Trust MCP policy forces manual approval for this tool.
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

    /// Spawn the Dual LLM verifier task if both verification is enabled and a
    /// provider/model pair is configured. Returns None if verification should
    /// be skipped (not enabled, not configured, or auto-approved).
    fn maybe_spawn_verifier(
        &self,
        name: &str,
        args: &serde_json::Map<String, Value>,
        security: &crate::config::models::SecurityConfig,
    ) -> Option<tokio::task::JoinHandle<VerificationOutcome>> {
        if !security.dual_llm_verification.unwrap_or(false) {
            return None;
        }

        let (v_provider, v_model) = self.ctx.config_manager.get_dual_llm_settings();

        if v_provider.is_empty() || v_model.is_empty() {
            self.ctx.ui.report_warning(
                "Dual LLM verification is enabled, but provider/model is not set. \
                 Falling back to manual approval.",
            );
            self.ctx
                .ui
                .report_info("Hint: Use /vp and /vm to set the verifier LLM.");
            None
        } else {
            Some(self.spawn_verifier_task(name, args, v_provider, v_model))
        }
    }

    /// Request human approval for a tool call. Returns Ok(Some(cancel_message)) if rejected
    /// with feedback (the caller should use this as the tool result and skip execution),
    /// Ok(None) if approved, or Err if interrupted.
    async fn request_human_approval(
        &mut self,
        name: &str,
        verifier_handle: Option<&tokio::task::JoinHandle<VerificationOutcome>>,
    ) -> anyhow::Result<Option<Value>> {
        match self.ctx.ui.ask_confirm(&format!("Execute {}", name)).await {
            Some(crate::cli::ui::ConfirmResult::Yes) => Ok(None),
            Some(res) => {
                if let Some(h) = verifier_handle {
                    h.abort();
                }
                let feedback = match res {
                    crate::cli::ui::ConfirmResult::Feedback(f) => Some(f),
                    _ => None,
                };
                let cancel_msg = self.handle_rejection_feedback(feedback)?;
                Ok(Some(cancel_msg))
            }
            None => {
                if let Some(h) = verifier_handle {
                    h.abort();
                }
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

    pub(crate) fn is_auto_approved(&self, _name: &str, risk: RiskLevel) -> bool {
        let config = match self.ctx.config_manager.get_config() {
            Ok(c) => c,
            Err(_) => return false,
        };
        let policy = config.security.auto_approval_level.unwrap_or_default();

        match policy {
            AutoApprovalLevel::Low if risk == RiskLevel::Low => {
                self.ctx.ui.report_success("Auto-approved (Low Risk)");
                true
            }
            AutoApprovalLevel::Medium if risk == RiskLevel::Low || risk == RiskLevel::Medium => {
                self.ctx.ui.report_success("Auto-approved (Medium Risk)");
                true
            }
            _ => false,
        }
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
                    // Take first HEAD_TAIL_CHARS chars and last HEAD_TAIL_CHARS chars,
                    // avoiding double-reversal: collect all chars once, then slice.
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
        // Truncate to MAX_TOTAL_CHARS using char_indices for O(1) boundary check
        match context.char_indices().nth(MAX_TOTAL_CHARS) {
            Some((cut_at, _)) => context[..cut_at].to_string(),
            None => context,
        }
    }

    /// Handle feedback from user when they reject a tool call.
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
}
