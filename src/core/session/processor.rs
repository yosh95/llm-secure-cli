use crate::cli::ui;
use crate::core::session::ActiveSession;
use crate::llm::models::{ContentPart, DataSource, Message, MessagePart, Role};
use crate::security::cass::RiskLevel;
use crate::security::dual_llm_verifier::{VerificationOutcome, VerificationParams};
use serde_json::{self, Value};
use std::collections::HashMap;
use std::io::Write;
use tokio;

impl ActiveSession {
    /// Main loop for processing user input and handling LLM interaction until no more tool calls or actions are required.
    pub async fn process_and_print(&mut self, data: Vec<DataSource>) -> anyhow::Result<()> {
        let mut current_data = data;

        loop {
            // 1. Send to LLM and get response
            let (text_response, thought) = self.think_and_receive(current_data).await?;
            current_data = Vec::new(); // Clear after use

            // 2. Output Thought and Response
            self.display_interaction_output(text_response, thought);

            // 3. Handle secondary outputs (Images, etc.)
            self.handle_media_output().await;

            // 4. Extract and handle Tool Calls
            let tool_results = self.handle_tool_calls().await?;

            if tool_results.is_empty() {
                break; // No more tools to execute
            } else {
                // Return tool results to the conversation
                self.client.get_state_mut().conversation.push(Message {
                    role: Role::Tool,
                    parts: tool_results,
                });
            }
        }
        Ok(())
    }

    /// Handles the "Thinking" state and API call to the LLM.
    async fn think_and_receive(
        &mut self,
        data: Vec<DataSource>,
    ) -> anyhow::Result<(Option<String>, Option<String>)> {
        let thinking_label = {
            let state = self.client.get_state();
            if state.provider.is_empty() {
                state.model.clone()
            } else {
                format!("{}/{}", state.provider, state.model)
            }
        };

        print!("Thinking ({}) ... ", thinking_label);
        std::io::stdout().flush().ok();

        let tool_schemas = if self.client.get_state().tools_enabled {
            self.ctx.tool_registry.lock().await.get_tool_schemas()
        } else {
            Vec::new()
        };
        let send_future = self.client.send(data, tool_schemas);

        let result = tokio::select! {
            res = send_future => res?,
            _ = tokio::signal::ctrl_c() => {
                println!("\n^C - Interrupted.");
                self.handle_interruption();
                return Err(anyhow::anyhow!("Interrupted by user"));
            }
        };

        println!("done");

        if let Some(usage) = &result.usage {
            self.total_usage.prompt_tokens += usage.prompt_tokens;
            self.total_usage.completion_tokens += usage.completion_tokens;
            self.total_usage.total_tokens += usage.total_tokens;

            use colored::*;
            println!(
                "{}",
                format!(
                    "Tokens: {} (↑) / {} (↓) / {} (Total)",
                    usage.prompt_tokens, usage.completion_tokens, usage.total_tokens
                )
                .dimmed()
            );
        }

        if let Some(redirect) = &result.tool_args {
            use colored::*;
            println!("{}", redirect.dimmed());
        }

        Ok((result.content, None))
    }

    /// Displays Thought blocks and Assistant text results.
    fn display_interaction_output(&self, text: Option<String>, thought: Option<String>) {
        if let Some(t) = thought
            && !t.trim().is_empty()
        {
            self.ctx
                .ui
                .print_rule(Some("Thought"), Some("bright_black"));
            self.ctx.ui.print_block(&t, None, Some("bright_black"));
            self.ctx.ui.print_rule(None, Some("bright_black"));
        }

        if let Some(text) = text
            && !text.trim().is_empty()
        {
            let (display_name, model) = (
                self.client.get_display_name(),
                self.client.get_state().model.clone(),
            );
            self.ctx
                .ui
                .print_block(&text, Some(&display_name), Some("cyan"));
            crate::utils::chat_logger::log_chat(
                &self.ctx.config_manager,
                &Role::Assistant,
                &text,
                Some(&model),
            );
        }
    }

    /// Processes images or other data types in the assistant's message.
    async fn handle_media_output(&self) {
        let config = match self.ctx.config_manager.get_config() {
            Ok(c) => c,
            Err(e) => {
                ui::report_error(&format!("Failed to load config for media output: {}", e));
                return;
            }
        };
        let last_msg = self.client.get_state().conversation.last();

        if let Some(msg) = last_msg
            && (msg.role == Role::Assistant || msg.role == Role::Model)
        {
            for part in &msg.parts {
                if let MessagePart::Part(cp) = part
                    && let Some(id) = &cp.inline_data
                {
                    let b64_data = id.get("data").and_then(|v| v.as_str()).unwrap_or("");
                    let mime_type = id.get("mimeType").and_then(|v| v.as_str()).unwrap_or("");
                    if !b64_data.is_empty() {
                        match crate::utils::media::save_media(
                            b64_data,
                            mime_type,
                            &config.general.image_save_path,
                        ) {
                            Ok(path) => self
                                .ctx
                                .ui
                                .report_success(&format!("Media saved to: {}", path)),
                            Err(e) => self
                                .ctx
                                .ui
                                .report_error(&format!("Failed to save media: {}", e)),
                        }
                    }
                }
            }
        }
    }

    /// Iterates through tool calls and manages their validation and execution.
    async fn handle_tool_calls(&mut self) -> anyhow::Result<Vec<MessagePart>> {
        let mut tool_results = Vec::new();
        // Clone to avoid borrow checker issues during loop
        let last_msg = self.client.get_state().conversation.last().cloned();

        if let Some(msg) = last_msg
            && (msg.role == Role::Assistant || msg.role == Role::Model)
        {
            for part in &msg.parts {
                if let MessagePart::Part(cp) = part
                    && let Some(fc) = &cp.function_call
                {
                    let name = fc.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let args = fc
                        .get("arguments")
                        .and_then(|v| v.as_object())
                        .cloned()
                        .unwrap_or_default();
                    let id = fc.get("id").and_then(|v| v.as_str()).unwrap_or("");

                    let (result_value, _approved) =
                        self.verify_and_execute_tool_workflow(name, &args).await?;

                    let mut fr = HashMap::new();
                    fr.insert("id".to_string(), serde_json::json!(id));
                    fr.insert("name".to_string(), serde_json::json!(name));
                    fr.insert("response".to_string(), result_value);

                    tool_results.push(MessagePart::Part(Box::new(ContentPart {
                        function_response: Some(fr),
                        is_diagnostic: false,
                        ..Default::default()
                    })));
                }
            }
        }
        Ok(tool_results)
    }

    /// The Multi-Phase Security Workflow for a single tool call.
    async fn verify_and_execute_tool_workflow(
        &mut self,
        name: &str,
        args: &serde_json::Map<String, Value>,
    ) -> anyhow::Result<(Value, bool)> {
        let config = self.ctx.config_manager.get_config()?;

        // 1. Phase 1: Static Checks (Simplified Safety Net)
        if let Err(e) = crate::security::validate_tool_call(name, args, &config.security) {
            self.ctx.ui.report_error(&e);
            return Ok((Value::String(e), false));
        }

        // 2. Human-in-the-loop preparation
        let risk_level = crate::security::cass::CASS_ORCHESTRATOR.evaluate_risk(
            name,
            Some(&serde_json::json!(args)),
            &config.security,
        );

        // Zero Trust Check for MCP Servers
        let mut force_manual = false;
        if name.contains("__") {
            let server_name = name.split("__").next().unwrap_or("");
            if let Some(mcp_config) = config.mcp_servers.iter().find(|s| s.name == server_name)
                && mcp_config.zero_trust
            {
                force_manual = true;
                self.ctx.ui.report_info(&format!(
                    "Zero Trust Policy enabled for server '{}'.",
                    server_name
                ));
            }
        }

        let approved = if force_manual {
            false
        } else {
            self.is_auto_approved(name, risk_level)
        };

        self.ctx.ui.print_tool_call(name, &serde_json::json!(args));

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

        // Start Dual LLM Verifier background task if enabled
        let mut verifier_handle = None;
        if !approved && config.security.dual_llm_verification.unwrap_or(false) {
            let (v_provider, v_model) = self.ctx.config_manager.get_dual_llm_settings();

            if v_provider.is_empty() || v_model.is_empty() {
                self.ctx.ui.report_warning(
                    "Dual LLM verification is enabled, but provider/model is not set. Falling back to manual approval.",
                );
                self.ctx
                    .ui
                    .report_info("Hint: Use /vp and /vm to set the verifier LLM.");
            } else {
                verifier_handle = Some(self.spawn_verifier_task(name, args, v_provider, v_model));
            }
        }

        // Request Human Approval if not auto-approved
        if !approved {
            match self.ctx.ui.ask_confirm(&format!("Execute {}", name)).await {
                Some(crate::cli::ui::ConfirmResult::Yes) => {
                    // Continue to verifier or execution
                }
                Some(res) => {
                    if let Some(h) = verifier_handle {
                        h.abort();
                    }
                    let feedback = match res {
                        crate::cli::ui::ConfirmResult::Feedback(f) => Some(f),
                        _ => None,
                    };
                    return self.handle_rejection_feedback(feedback).map(|v| (v, false));
                }
                None => {
                    if let Some(h) = verifier_handle {
                        h.abort();
                    }
                    self.handle_interruption();
                    return Err(anyhow::anyhow!("Interrupted"));
                }
            }
        }

        let mut effective_args = args.clone();

        // 3. Resolve Dual LLM Verification
        if let Some(handle) = verifier_handle {
            let outcome = self.resolve_verifier_outcome(handle).await?;
            match outcome {
                VerificationOutcome::Allowed(reason) => {
                    self.ctx
                        .ui
                        .report_success(&format!("Intent Verified: {}", reason));
                }
                VerificationOutcome::Modified(fixed_args, reason) => {
                    self.ctx
                        .ui
                        .report_success(&format!("Intent Verified & Corrected: {}", reason));
                    if let Some(obj) = fixed_args.as_object() {
                        effective_args = obj.clone();
                    }
                }
                VerificationOutcome::Rejected(reason) => {
                    let msg = format!("Security Policy Violation: {}", reason);
                    self.ctx.ui.report_error(&msg);
                    return Ok((Value::String(msg), false));
                }
                VerificationOutcome::FallbackRequired(reason) => {
                    if !self.handle_verifier_fallback(name, &reason).await {
                        return Ok((
                            Value::String(format!("Blocked (Verifier Unavailable): {}", reason)),
                            false,
                        ));
                    }
                }
            }
        }

        // 4. Execution & Audit
        let result = self
            .execute_and_audit_tool(name, &effective_args, risk_level, approved)
            .await;
        Ok((result, approved))
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

    /// Internal execution and audit logging logic.
    async fn execute_and_audit_tool(
        &mut self,
        name: &str,
        args: &serde_json::Map<String, Value>,
        _risk_level: RiskLevel,
        approved: bool,
    ) -> Value {
        let config = match self.ctx.config_manager.get_config() {
            Ok(c) => c,
            Err(e) => return Value::String(format!("Error: Failed to load config: {}", e)),
        };
        let user_id = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());

        let result = tokio::select! {
            res = self.execute_tool(name, args.clone().into_iter().collect()) => res,
            _ = tokio::signal::ctrl_c() => {
                println!("\n^C - Interrupted.");
                // We return an error string to the LLM
                return Value::String("Error: Execution interrupted by user.".into());
            }
        };

        let audit_ctx = serde_json::json!({
            "trace_id": self.trace_id,
            "model": self.client.get_state().model.clone(),
            "provider": self.client.get_state().provider.clone(),
            "user_id": user_id
        });

        let mut is_error = false;
        let mut final_v = match result {
            Ok(v) => {
                let entry = tokio::task::block_in_place(|| {
                    crate::security::audit::log_audit_and_return(
                        crate::security::audit::AuditParams {
                            event_type: "tool_call",
                            tool_name: name,
                            args: serde_json::json!(args),
                            output: v.as_str(),
                            exit_code: Some(0),
                            error: None,
                            context: Some(&audit_ctx),
                            config: &config,
                        },
                        None,
                    )
                });
                if let Some(entry) = entry {
                    self.audit_entries.push(entry);
                }
                v
            }
            Err(e) => {
                is_error = true;
                let err_msg = e.to_string();
                let entry = tokio::task::block_in_place(|| {
                    crate::security::audit::log_audit_and_return(
                        crate::security::audit::AuditParams {
                            event_type: "tool_call",
                            tool_name: name,
                            args: serde_json::json!(args),
                            output: None,
                            exit_code: Some(1),
                            error: Some(&err_msg),
                            context: Some(&audit_ctx),
                            config: &config,
                        },
                        None,
                    )
                });
                if let Some(entry) = entry {
                    self.audit_entries.push(entry);
                }
                Value::String(format!("Error: {}", err_msg))
            }
        };

        crate::tools::executor_utils::truncate_json_strings(&mut final_v);

        // Calculate and display stats
        let stats = crate::cli::stats::get_tool_result_stats(&final_v);

        // Display result if it was not auto-approved OR if an error occurred.
        // This ensures that auto-approved successful calls don't clutter the UI with output (like stdout),
        // but failures are always shown. The tool call itself is always printed above.
        if !approved || is_error {
            self.ctx
                .ui
                .print_tool_result(final_v.as_str().unwrap_or(&final_v.to_string()));

            // If we already printed common tool result UI, we don't want to re-print stderr in stats
            let mut quiet_stats = stats.clone();
            quiet_stats.stderr = None;
            crate::cli::stats::print_tool_stats(&quiet_stats);
        } else {
            // Even if auto-approved, we show stats and stderr if present
            crate::cli::stats::print_tool_stats(&stats);
        }

        // Convert to human-readable string for the LLM
        let human_result = crate::tools::executor_utils::humanize_tool_result(name, &final_v);
        Value::String(human_result)
    }

    fn is_auto_approved(&self, _name: &str, risk: RiskLevel) -> bool {
        let config = match self.ctx.config_manager.get_config() {
            Ok(c) => c,
            Err(_) => return false,
        };
        let policy = config
            .security
            .auto_approval_level
            .as_deref()
            .unwrap_or("none");

        match policy {
            "low" if risk == RiskLevel::Low => {
                self.ctx.ui.report_success("Auto-approved (Low Risk)");
                true
            }
            "medium" if risk == RiskLevel::Low || risk == RiskLevel::Medium => {
                self.ctx.ui.report_success("Auto-approved (Medium Risk)");
                true
            }
            _ => false,
        }
    }

    async fn handle_verifier_fallback(&self, name: &str, reason: &str) -> bool {
        let config = match self.ctx.config_manager.get_config() {
            Ok(c) => c,
            Err(_) => return false,
        };
        match config.security.verifier_fallback.as_str() {
            "block" => {
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

    fn handle_rejection_feedback(&mut self, feedback: Option<String>) -> anyhow::Result<Value> {
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

    fn get_intent_context(&self) -> String {
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
                if text.chars().count() > 1000 {
                    let head: String = text.chars().take(500).collect();
                    let tail: String = text.chars().rev().take(500).collect::<String>();
                    format!(
                        "{}...[TRUNCATED]...{}",
                        head,
                        tail.chars().rev().collect::<String>()
                    )
                } else {
                    text
                }
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>();

        let context = history.join("\n---\n");
        if context.chars().count() > 4000 {
            context
                .chars()
                .rev()
                .take(4000)
                .collect::<String>()
                .chars()
                .rev()
                .collect()
        } else {
            context
        }
    }
}
