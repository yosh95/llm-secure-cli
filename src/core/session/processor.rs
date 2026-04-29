use crate::cli::ui;
use crate::core::session::ChatSession;
use crate::llm::models::{ContentPart, DataSource, Message, MessagePart, Role};
use crate::security::cass::RiskLevel;
use indicatif::{ProgressBar, ProgressStyle};
use serde_json;
use std::collections::HashMap;
use tokio;

impl ChatSession {
    pub async fn process_and_print(&mut self, data: Vec<DataSource>) -> anyhow::Result<()> {
        let mut current_data = data;
        let user_id = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());

        loop {
            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::default_spinner()
                    .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
                    .template("{spinner:.cyan} {msg}")?,
            );
            pb.set_message(format!("Thinking... ({})", self.client.get_state().model));
            pb.enable_steady_tick(std::time::Duration::from_millis(100));

            let (response, thought) = self.client.send(current_data).await?;
            pb.finish_and_clear();

            current_data = Vec::new();

            if let Some(t) = thought
                && !t.trim().is_empty()
            {
                ui::print_rule(Some("Thought"), Some("bright_black"));
                ui::print_block(&t, None, Some("bright_black"));
                ui::print_rule(None, Some("bright_black"));
            }

            if let Some(text) = response
                && !text.trim().is_empty()
            {
                ui::print_block(&text, Some(&self.client.get_display_name()), Some("cyan"));
                crate::utils::chat_logger::log_chat(&crate::llm::models::Role::Assistant, &text);
            }

            // Handle incoming images
            let last_msg = self.client.get_state().conversation.last().cloned();
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
                            let config = crate::config::CONFIG_MANAGER.get_config();
                            match crate::utils::media::save_image(
                                b64_data,
                                mime_type,
                                &config.general.image_save_path,
                            ) {
                                Ok(path) => {
                                    ui::report_success(&format!("Image saved to: {}", path))
                                }
                                Err(e) => ui::report_error(&format!("Failed to save image: {}", e)),
                            }
                        }
                    }
                }
            }

            // Handle tool calls
            let mut tool_results = Vec::new();
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

                        ui::print_tool_call(name, &serde_json::json!(args));

                        let mut final_result = None;

                        // --- [PHASE 1] Lightweight Fast Checks ---
                        if let Err(e) =
                            crate::security::validate_tool_call(name, &args, &self.config.security)
                        {
                            ui::report_error(&e);
                            final_result = Some(serde_json::Value::String(e));
                        }

                        // --- [PHASE 2] High-Assurance Checks ---
                        let _verifier_result: Option<(bool, String)> = None;
                        let mut verifier_handle: Option<tokio::task::JoinHandle<(bool, String)>> =
                            None;

                        if final_result.is_none()
                            && self.config.security.dual_llm_verification.unwrap_or(false)
                        {
                            // Build intent context for verification
                            let user_history: Vec<String> = self
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
                                        let tail: String = text
                                            .chars()
                                            .rev()
                                            .take(500)
                                            .collect::<String>()
                                            .chars()
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
                                .collect();

                            let mut intent_context = user_history.join("\n---\n");
                            if intent_context.chars().count() > 4000 {
                                intent_context = intent_context
                                    .chars()
                                    .rev()
                                    .take(4000)
                                    .collect::<String>()
                                    .chars()
                                    .rev()
                                    .collect();
                            }

                            let name_clone = name.to_string();
                            let args_clone = serde_json::json!(args);
                            let config_clone = self.config.security.clone();
                            verifier_handle = Some(tokio::spawn(async move {
                                crate::security::dual_llm_verifier::verify_tool_call_full(
                                    &intent_context,
                                    &name_clone,
                                    &args_clone,
                                    None,
                                    &config_clone,
                                    None,
                                    None,
                                )
                                .await
                            }));
                        }

                        // Risk evaluation & auto-approval
                        let risk_level = crate::security::cass::CASS_ORCHESTRATOR
                            .evaluate_risk(name, &self.config.security);
                        let auto_approval = self
                            .config
                            .security
                            .auto_approval_level
                            .as_deref()
                            .unwrap_or("none");

                        let mut approved = false;
                        if auto_approval == "low" && risk_level == RiskLevel::Low {
                            approved = true;
                            ui::report_success("Auto-approved (Low Risk)");
                        } else if auto_approval == "medium"
                            && (risk_level == RiskLevel::Low || risk_level == RiskLevel::Medium)
                        {
                            approved = true;
                            ui::report_success("Auto-approved (Medium Risk)");
                        }

                        if !approved {
                            match ui::ask_confirm(&format!("Execute {}", name)) {
                                Some(val) => approved = val,
                                None => {
                                    // User hit Ctrl+C: abort the background verifier task
                                    // before returning so it does not keep running.
                                    if let Some(handle) = verifier_handle.take() {
                                        handle.abort();
                                    }
                                    self.handle_interruption();
                                    return Ok(());
                                }
                            }
                        }

                        if !approved {
                            ui::report_warning("Execution cancelled by user.");
                            match ui::get_user_input("Provide feedback (optional): ") {
                                Some(feedback) => {
                                    let result_msg = if feedback.trim().is_empty() {
                                        "Error: Execution cancelled by user.".to_string()
                                    } else {
                                        format!(
                                            "Error: Execution cancelled by user. Feedback: {}",
                                            feedback
                                        )
                                    };
                                    // Abort the background verifier task since we will
                                    // not be awaiting it anymore.
                                    if let Some(handle) = verifier_handle.take() {
                                        handle.abort();
                                    }
                                    final_result = Some(serde_json::Value::String(result_msg));
                                }
                                None => {
                                    // User hit Ctrl+C during feedback prompt.
                                    if let Some(handle) = verifier_handle.take() {
                                        handle.abort();
                                    }
                                    self.handle_interruption();
                                    return Ok(());
                                }
                            }
                        }

                        // Resolve verification result
                        if final_result.is_none()
                            && let Some(handle) = verifier_handle
                        {
                            let pb_v = ProgressBar::new_spinner();
                            pb_v.set_style(
                                ProgressStyle::default_spinner()
                                    .template("{spinner:.yellow} {msg}")?,
                            );
                            pb_v.set_message("Finalizing intent verification...");
                            pb_v.enable_steady_tick(std::time::Duration::from_millis(100));

                            let (safe, reason) = handle.await.unwrap_or_else(|_| {
                                (false, "Verification task panicked".to_string())
                            });
                            pb_v.finish_and_clear();

                            if !safe {
                                ui::report_error(&format!(
                                    "Dual LLM Verification failed: {}",
                                    reason
                                ));
                                final_result = Some(serde_json::Value::String(format!(
                                    "Security Policy Violation: {}",
                                    reason
                                )));
                            } else {
                                ui::report_success(&format!("Intent Verified: {}", reason));
                            }
                        }

                        // --- [PHASE 3] Execution ---
                        let result_value = if let Some(res) = final_result {
                            res
                        } else {
                            let result = self
                                .execute_tool(name, args.clone().into_iter().collect())
                                .await;
                            let audit_ctx = serde_json::json!({
                                "trace_id": self.trace_id,
                                "model": self.client.get_state().model,
                                "user_id": user_id
                            });
                            match result {
                                Ok(v) => {
                                    if let Some(entry) =
                                        crate::security::audit::log_audit_and_return(
                                            "tool_call",
                                            name,
                                            serde_json::json!(args),
                                            v.as_str(),
                                            Some(0),
                                            None,
                                            Some(&audit_ctx),
                                            &self.config,
                                            None,
                                        )
                                    {
                                        self.audit_entries.push(entry);
                                    }
                                    v
                                }
                                Err(e) => {
                                    if let Some(entry) =
                                        crate::security::audit::log_audit_and_return(
                                            "tool_call",
                                            name,
                                            serde_json::json!(args),
                                            None,
                                            Some(1),
                                            Some(&e.to_string()),
                                            Some(&audit_ctx),
                                            &self.config,
                                            None,
                                        )
                                    {
                                        self.audit_entries.push(entry);
                                    }
                                    serde_json::Value::String(format!("Error: {}", e))
                                }
                            }
                        };

                        ui::print_tool_result(
                            result_value.as_str().unwrap_or(&result_value.to_string()),
                        );

                        let mut fr = HashMap::new();
                        fr.insert("id".to_string(), serde_json::json!(id));
                        fr.insert("name".to_string(), serde_json::json!(name));
                        fr.insert("response".to_string(), result_value);

                        tool_results.push(MessagePart::Part(ContentPart {
                            text: None,
                            inline_data: None,
                            function_call: None,
                            function_response: Some(fr),
                            thought: None,
                            thought_signature: None,
                            is_diagnostic: false,
                        }));
                    }
                }
            }

            if tool_results.is_empty() {
                break;
            } else {
                self.client.get_state_mut().conversation.push(Message {
                    role: Role::Tool,
                    parts: tool_results,
                });
                // After adding tool results to history, we need to send the full
                // conversation context rather than chaining with previous_interaction_id,
                // since the new data vector is empty and the server needs to see the
                // tool results in the conversation turns.
                self.client.get_state_mut().previous_interaction_id = None;
            }
        }
        Ok(())
    }
}
