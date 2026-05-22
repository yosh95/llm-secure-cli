use crate::cli::ui;
use crate::core::session::ActiveSession;
use crate::llm::models::{ContentPart, DataSource, Message, MessagePart, Role};
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
                // Auto-save session after each complete turn
                crate::utils::session_store::auto_save(self);
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
            self.ctx.tool_registry.read().await.get_tool_schemas()
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
            let display_name = self.client.get_display_name();
            self.ctx
                .ui
                .print_block(&text, Some(&display_name), Some("cyan"));
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
                    let b64_data = id.get("data").and_then(|v| v.as_str()).unwrap_or_default();
                    let mime_type = id
                        .get("mimeType")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();
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
                    let name = fc.get("name").and_then(|v| v.as_str()).unwrap_or_default();
                    let args = fc
                        .get("arguments")
                        .and_then(|v| v.as_object())
                        .cloned()
                        .unwrap_or_default();
                    let id = fc.get("id").and_then(|v| v.as_str()).unwrap_or_default();

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
    /// Delegates to four distinct phases for clarity and maintainability.
    async fn verify_and_execute_tool_workflow(
        &mut self,
        name: &str,
        args: &serde_json::Map<String, Value>,
    ) -> anyhow::Result<(Value, bool)> {
        let config = self.ctx.config_manager.get_config()?;

        // Phase 1: Static analysis — fast-fail deterministic checks
        self.phase1_static_check(name, args, &config)?;

        // Phase 2: Risk assessment, Zero Trust, approval gate
        let (risk_level, approved, verifier_handle, cancel_msg) =
            self.phase2_risk_and_approval(name, args, &config).await?;

        // If the user cancelled with feedback, return the cancel message directly
        if let Some(msg) = cancel_msg {
            return Ok((msg, false));
        }

        // Phase 3: Dual LLM semantic verification
        let effective_args = self
            .phase3_dual_llm_verification(name, args, verifier_handle)
            .await?;

        // Phase 4: Execution and audit logging
        let result = self
            .phase4_execute_and_audit(name, &effective_args, risk_level, approved)
            .await;
        Ok((result, approved))
    }
}
