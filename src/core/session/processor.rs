use crate::cli::ui;
use crate::core::session::ChatSession;
use crate::llm::models::{ContentPart, DataSource, Message, MessagePart, Role};
use crate::security::cass::RiskLevel;
use crate::security::dual_llm_verifier::{VerificationOutcome, VerificationParams};
use serde_json::{self, Value};
use std::collections::HashMap;
use std::io::Write;
use tokio;

impl ChatSession {
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
                self.get_client_mut()
                    .get_state_mut()
                    .conversation
                    .push(Message {
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
            let state = self.get_client().get_state();
            if state.provider.is_empty() {
                state.model.clone()
            } else {
                format!("{}/{}", state.provider, state.model)
            }
        };

        print!("Thinking ({}) ... ", thinking_label);
        std::io::stdout().flush().ok();

        let tool_schemas = self.ctx.tool_registry.lock().await.get_tool_schemas();
        let send_future = self.get_client_mut().send(data, tool_schemas);

        let result = tokio::select! {
            res = send_future => res?,
            _ = tokio::signal::ctrl_c() => {
                println!("\n^C - Interrupted.");
                self.handle_interruption();
                return Err(anyhow::anyhow!("Interrupted by user"));
            }
        };

        println!("done");
        Ok(result)
    }

    /// Displays Thought blocks and Assistant text results.
    fn display_interaction_output(&self, text: Option<String>, thought: Option<String>) {
        if let Some(t) = thought
            && !t.trim().is_empty()
        {
            ui::print_rule(Some("Thought"), Some("bright_black"));
            ui::print_block(&t, None, Some("bright_black"));
            ui::print_rule(None, Some("bright_black"));
        }

        if let Some(text) = text
            && !text.trim().is_empty()
        {
            ui::print_block(
                &text,
                Some(&self.get_client().get_display_name()),
                Some("cyan"),
            );
            crate::utils::chat_logger::log_chat(
                &self.ctx.config_manager,
                &Role::Assistant,
                &text,
                Some(&self.get_client().get_state().model),
            );
        }
    }

    /// Processes images or other data types in the assistant's message.
    async fn handle_media_output(&self) {
        let config = self.ctx.config_manager.get_config();
        let last_msg = self.get_client().get_state().conversation.last();

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
                        match crate::utils::media::save_image(
                            b64_data,
                            mime_type,
                            &config.general.image_save_path,
                        ) {
                            Ok(path) => ui::report_success(&format!("Image saved to: {}", path)),
                            Err(e) => ui::report_error(&format!("Failed to save image: {}", e)),
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
        let last_msg = self.get_client().get_state().conversation.last().cloned();

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

                    ui::print_tool_call(name, &serde_json::json!(args));

                    let result_value = self.verify_and_execute_tool_workflow(name, &args).await?;

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
    ) -> anyhow::Result<Value> {
        let config = self.ctx.config_manager.get_config();

        // 1. Phase 1: Static Checks (Path, Syntax)
        if let Err(e) = crate::security::validate_tool_call(name, args, &config.security) {
            ui::report_error(&e);
            return Ok(Value::String(e));
        }

        // 2. Phase 2 & Human-in-the-loop preparation
        let risk_level =
            crate::security::cass::CASS_ORCHESTRATOR.evaluate_risk(name, &config.security);
        let approved = self.is_auto_approved(name, risk_level);

        // Start Dual LLM Verifier background task if enabled
        let mut verifier_handle = None;
        if !approved && config.security.dual_llm_verification.unwrap_or(false) {
            verifier_handle = Some(self.spawn_verifier_task(name, args));
        }

        // Request Human Approval if not auto-approved
        if !approved && ui::ask_confirm_async(&format!("Execute {}", name)).await != Some(true) {
            if let Some(h) = verifier_handle {
                h.abort();
            }
            return self.handle_rejection_feedback();
        }

        // 3. Resolve Dual LLM Verification
        if let Some(handle) = verifier_handle {
            let outcome = self.resolve_verifier_outcome(handle).await?;
            match outcome {
                VerificationOutcome::Allowed(reason) => {
                    ui::report_success(&format!("Intent Verified: {}", reason));
                }
                VerificationOutcome::Rejected(reason) => {
                    let msg = format!("Security Policy Violation: {}", reason);
                    ui::report_error(&msg);
                    return Ok(Value::String(msg));
                }
                VerificationOutcome::FallbackRequired(reason) => {
                    if !self.handle_verifier_fallback(name, &reason).await {
                        return Ok(Value::String(format!(
                            "Blocked (Verifier Unavailable): {}",
                            reason
                        )));
                    }
                }
            }
        }

        // 4. Execution & Audit
        let result = self.execute_and_audit_tool(name, args, risk_level).await;
        Ok(result)
    }

    /// Spawns the Dual LLM Verification task.
    fn spawn_verifier_task(
        &self,
        name: &str,
        args: &serde_json::Map<String, Value>,
    ) -> tokio::task::JoinHandle<VerificationOutcome> {
        let ctx_clone = self.ctx.clone();
        let config_clone = self.ctx.config_manager.get_config().security.clone();
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
                provider: None,
                model: None,
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

        let res = tokio::select! {
            res = handle => res.unwrap_or(VerificationOutcome::FallbackRequired("Task Panicked".into())),
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
    ) -> Value {
        let config = self.ctx.config_manager.get_config();
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
            "model": self.get_client().get_state().model,
            "user_id": user_id
        });

        let mut final_v = match result {
            Ok(v) => {
                if let Some(entry) = crate::security::audit::log_audit_and_return(
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
                ) {
                    self.audit_entries.push(entry);
                }
                v
            }
            Err(e) => {
                let err_msg = e.to_string();
                if let Some(entry) = crate::security::audit::log_audit_and_return(
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
                ) {
                    self.audit_entries.push(entry);
                }
                Value::String(format!("Error: {}", err_msg))
            }
        };

        crate::tools::executor_utils::truncate_json_strings(&mut final_v);
        ui::print_tool_result(final_v.as_str().unwrap_or(&final_v.to_string()));
        final_v
    }

    fn is_auto_approved(&self, _name: &str, risk: RiskLevel) -> bool {
        let config = self.ctx.config_manager.get_config();
        let policy = config
            .security
            .auto_approval_level
            .as_deref()
            .unwrap_or("none");

        match policy {
            "low" if risk == RiskLevel::Low => {
                ui::report_success("Auto-approved (Low Risk)");
                true
            }
            "medium" if risk == RiskLevel::Low || risk == RiskLevel::Medium => {
                ui::report_success("Auto-approved (Medium Risk)");
                true
            }
            _ => false,
        }
    }

    async fn handle_verifier_fallback(&self, name: &str, reason: &str) -> bool {
        let config = self.ctx.config_manager.get_config();
        match config.security.verifier_fallback.as_str() {
            "block" => {
                ui::report_error(&format!("Verifier unavailable — blocked: {}", reason));
                false
            }
            _ => {
                ui::report_warning(&format!("⚠ Verifier unavailable: {}", reason));
                ui::ask_confirm_async(&format!("Execute {} (Manual confirmation required)", name))
                    .await
                    .unwrap_or(false)
            }
        }
    }

    fn handle_rejection_feedback(&mut self) -> anyhow::Result<Value> {
        ui::report_warning("Execution cancelled by user.");
        match ui::get_user_input("Provide feedback (optional): ") {
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
            .get_client()
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
            .collect();

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
