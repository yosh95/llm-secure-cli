use crate::cli::ui;
use crate::core::session::ActiveSession;
use crate::llm::models::{ContentPart, DataSource, Message, MessagePart, Role};
use serde_json::{self, Value};
use std::collections::HashMap;

impl ActiveSession {
    /// Main loop for processing user input and handling LLM interaction until no more tool calls or actions are required.
    pub fn process_and_print(&mut self, data: Vec<DataSource>) -> anyhow::Result<()> {
        let mut current_data = data;

        loop {
            // 1. Send to LLM and get response
            let (text_response, thought) = self.think_and_receive(current_data)?;
            current_data = Vec::new(); // Clear after use

            // 2. Output Thought and Response
            self.display_interaction_output(text_response, thought);

            // 3. Handle secondary outputs (Images, etc.)
            self.handle_media_output();

            // 4. Extract and handle Tool Calls
            let tool_results = self.handle_tool_calls()?;

            if tool_results.is_empty() {
                // Auto-save session after each complete turn
                crate::utils::session_store::auto_save(self);
                break; // No more tools to execute
            }
            // Return tool results to the conversation
            self.client.get_state_mut().conversation.push(Message {
                role: Role::Tool,
                parts: tool_results,
            });
        }
        Ok(())
    }

    /// Handles the "Thinking" state and API call to the LLM.
    fn think_and_receive(
        &mut self,
        data: Vec<DataSource>,
    ) -> anyhow::Result<(Option<String>, Option<String>)> {
        let tool_schemas = if self.client.get_state().tools_enabled {
            self.ctx
                .tool_registry
                .read()
                .unwrap_or_else(|p| p.into_inner())
                .get_tool_schemas()
        } else {
            Vec::new()
        };

        let is_stdout = self.client.get_state().stdout;

        if !is_stdout {
            let provider = &self.client.get_state().provider;
            let model = &self.client.get_state().model;
            self.ctx
                .ui
                .report_info(&format!("LLM: {}:{} (querying)...", provider, model,));
        }

        // The blocking HTTP request inside `send` runs through `run_cancellable`,
        // so Ctrl+C surfaces as an error.  In interactive mode, detect that the
        // failure was caused by a Ctrl+C and clean up the interrupted turn.
        let result = if is_stdout {
            self.client.send(data, tool_schemas)?
        } else {
            let cancel_gen = self.cancel_token.generation();
            match self.client.send(data, tool_schemas) {
                Ok(r) => r,
                Err(e) => {
                    if crate::core::session::cancelled_since(cancel_gen) {
                        eprintln!("^C - Interrupted.");
                        self.handle_interruption();
                        return Err(anyhow::anyhow!("Interrupted by user"));
                    }
                    return Err(e);
                }
            }
        };

        Ok((result.content, None))
    }

    /// Displays Thought blocks and Assistant text results.
    fn display_interaction_output(&self, text: Option<String>, thought: Option<String>) {
        let is_stdout = self.client.get_state().stdout;

        if is_stdout {
            // In stdout/pipe mode: print only the raw response text, no ANSI, no markdown, no decoration
            if let Some(text) = text
                && !text.trim().is_empty()
            {
                print!("{}", text);
                if !text.ends_with('\n') {
                    println!();
                }
                use std::io::Write;
                std::io::stdout().flush().ok();
            }
            return;
        }

        if let Some(t) = thought
            && !t.trim().is_empty()
        {
            self.ctx.ui.print_rule(Some("Thought"), Some("cyan"));
            self.ctx.ui.print_block(&t, None, Some("cyan"));
            self.ctx.ui.print_rule(None, Some("cyan"));
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
    fn handle_media_output(&self) {
        if self.client.get_state().stdout {
            return;
        }
        let config = match self.ctx.config_manager.get_config() {
            Ok(c) => c,
            Err(e) => {
                ui::report_error(&format!("Failed to load config for media output: {e}"));
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
                                .report_success(&format!("Media saved to: {path}")),
                            Err(e) => self
                                .ctx
                                .ui
                                .report_error(&format!("Failed to save media: {e}")),
                        }
                    }
                }
            }
        }
    }

    /// Iterates through tool calls and manages their validation and execution.
    fn handle_tool_calls(&mut self) -> anyhow::Result<Vec<MessagePart>> {
        let mut tool_results = Vec::new();
        // Clone to avoid borrow checker issues during loop
        let last_msg = self.client.get_state().conversation.last().cloned();

        // Capture cancellation baseline to detect Ctrl+C between tool calls.
        // When the generation differs, a Ctrl+C has occurred and we abort remaining calls.
        let cancel_base = self.cancel_token.generation();

        if let Some(msg) = last_msg
            && (msg.role == Role::Assistant || msg.role == Role::Model)
        {
            for part in &msg.parts {
                // If user pressed Ctrl+C between tool calls, abort immediately.
                if crate::core::session::cancelled_since(cancel_base) {
                    self.handle_interruption();
                    return Err(anyhow::anyhow!("Interrupted by user"));
                }

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

                    let result_value = match self.verify_and_execute_tool_workflow(name, &args) {
                        Ok((val, _)) => val,
                        Err(e) => {
                            let err_msg = format!("Error (Phase 1): {e}");
                            self.ctx.ui.report_error(&err_msg);
                            serde_json::json!({"error": err_msg})
                        }
                    };

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
    /// Delegates to three distinct phases for clarity and maintainability.
    fn verify_and_execute_tool_workflow(
        &mut self,
        name: &str,
        args: &serde_json::Map<String, Value>,
    ) -> anyhow::Result<(Value, bool)> {
        let config = self.ctx.config_manager.get_config()?;

        // Phase 1: Static analysis — fast-fail deterministic checks
        self.phase1_static_check(name, args, &config)?;

        // Phase 2: Verification & Approval — Zero Trust, Verifier LLM, human-in-the-loop
        let (effective_args, auto_approved, cancel_msg) =
            self.phase2_verification(name, args, &config)?;

        // If the user cancelled with feedback, return the cancel message directly
        if let Some(msg) = cancel_msg {
            return Ok((msg, false));
        }

        // Phase 3: Execution and audit logging
        let result = self.phase3_execute_and_audit(name, &effective_args, auto_approved);
        Ok((result, auto_approved))
    }
}
