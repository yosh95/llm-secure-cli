use crate::cli::interactive::completion::ChatCompleter;
use crate::cli::ui;
use crate::consts::HISTORY_LOG_PATH;
use crate::llm::base::LlmClient;
use crate::llm::models::{ContentPart, DataSource, Message, MessagePart, Role};
use rustyline::error::ReadlineError;
use rustyline::history::FileHistory;
use rustyline::Editor;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub struct ChatSession {
    pub client: Box<dyn LlmClient>,
    pub intent: String,
    pub pending_data: Vec<DataSource>,
}

impl ChatSession {
    pub fn new(client: Box<dyn LlmClient>) -> Self {
        Self {
            client,
            intent: String::new(),
            pending_data: Vec::new(),
        }
    }

    pub fn switch_client(&mut self, mut new_client: Box<dyn LlmClient>) {
        {
            let old_state = self.client.get_state();
            let new_state = new_client.get_state_mut();
            new_state.conversation = old_state.conversation.clone();
            new_state.live_debug = old_state.live_debug;
            // Only carry over tools_enabled if the new model supports tools by default.
            // If the new model has tools explicitly disabled in config, respect that.
            if new_state.tools_enabled {
                new_state.tools_enabled = old_state.tools_enabled;
            }
            new_state.system_prompt_enabled = old_state.system_prompt_enabled;
        }
        self.client = new_client;
    }

    pub async fn run(
        &mut self,
        initial_data: Option<Vec<DataSource>>,
        _sources: Option<Vec<String>>,
    ) {
        let data = initial_data.unwrap_or_default();
        let is_stdout = self.client.get_state().stdout;

        if !data.is_empty() {
            if self.intent.is_empty() {
                // If we have initial data (from CLI args), use the first text part as intent
                if let Some(DataSource {
                    content: serde_json::Value::String(s),
                    ..
                }) = data.first()
                {
                    self.intent = s.clone();
                }
            }

            match self.process_and_print(data).await {
                Ok(_) => {
                    if is_stdout {
                        return;
                    }
                }
                Err(e) => {
                    ui::report_error(&format!("Error: {}", e));
                    if is_stdout {
                        return;
                    }
                }
            }
        }

        if is_stdout {
            // If we reached here in stdout mode without data, it's an error in main.rs logic
            // but we just return to be safe.
            return;
        }

        println!("Use Ctrl+C or /q to exit, /h for help.");

        let current_provider = Arc::new(Mutex::new(self.client.get_state().provider.clone()));
        let mut rl = Editor::<ChatCompleter, FileHistory>::new().expect("Failed to create editor");
        rl.set_helper(Some(ChatCompleter::new(current_provider.clone())));

        if let Some(parent) = HISTORY_LOG_PATH.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = rl.load_history(&*HISTORY_LOG_PATH);

        let mut line_buffer = Vec::new();
        let mut next_initial_text: Option<String> = None;

        loop {
            // Update current provider in completer in case it changed
            {
                let mut cp = current_provider.lock().unwrap();
                *cp = self.client.get_state().provider.clone();
            }

            let readline = if let Some(initial) = next_initial_text.take() {
                rl.readline_with_initial("> ", (&initial, ""))
            } else {
                let prompt = if line_buffer.is_empty() { "> " } else { ">> " };
                rl.readline(prompt)
            };

            match readline {
                Ok(line) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() && line_buffer.is_empty() {
                        continue;
                    }

                    // Check for line continuation
                    if let Some(stripped) = trimmed.strip_suffix('\\') {
                        line_buffer.push(stripped.to_string());
                        continue;
                    }

                    let full_line = if line_buffer.is_empty() {
                        trimmed.to_string()
                    } else {
                        line_buffer.push(trimmed.to_string());
                        let combined = line_buffer.join("\n");
                        line_buffer.clear();
                        combined
                    };

                    let final_trimmed = full_line.trim();
                    if final_trimmed.is_empty() {
                        continue;
                    }

                    // Handle commands
                    let (content, should_continue) =
                        match crate::cli::interactive::dispatcher::handle_command(
                            self,
                            final_trimmed,
                        )
                        .await
                        {
                            crate::cli::interactive::dispatcher::CommandResult::Exit => break,
                            crate::cli::interactive::dispatcher::CommandResult::Handled => {
                                let _ = rl.add_history_entry(final_trimmed);
                                (None, true)
                            }
                            crate::cli::interactive::dispatcher::CommandResult::NotACommand => {
                                (Some(final_trimmed.to_string()), false)
                            }
                            crate::cli::interactive::dispatcher::CommandResult::Input(text) => {
                                next_initial_text = Some(text);
                                (None, true)
                            }
                        };

                    if should_continue {
                        continue;
                    }

                    let _ = rl.add_history_entry(final_trimmed);
                    let final_content = content.unwrap_or_else(|| final_trimmed.to_string());

                    if self.intent.is_empty() {
                        self.intent = final_content.clone();
                    }

                    let mut data = std::mem::take(&mut self.pending_data);
                    data.push(DataSource {
                        content: serde_json::Value::String(final_content),
                        content_type: "text/plain".to_string(),
                        is_file_or_url: false,
                        metadata: std::collections::HashMap::new(),
                    });

                    let mut process_future = Box::pin(self.process_and_print(data));

                    match tokio::select! {
                        res = &mut process_future => res,
                        _ = tokio::signal::ctrl_c() => {
                            drop(process_future);
                            println!("\n^C - Interrupted. Returning to prompt...");
                            self.handle_interruption();
                            Ok(())
                        }
                    } {
                        Ok(_) => {}
                        Err(e) => {
                            ui::report_error(&format!("Error: {}", e));
                        }
                    }
                }
                Err(ReadlineError::Interrupted) => {
                    println!("CTRL-C");
                    break;
                }
                Err(ReadlineError::Eof) => {
                    println!("CTRL-D");
                    break;
                }
                Err(err) => {
                    ui::report_error(&format!("Error: {:?}", err));
                    break;
                }
            }
        }
        let _ = rl.save_history(&*HISTORY_LOG_PATH);
    }

    pub async fn process_and_print(&mut self, data: Vec<DataSource>) -> anyhow::Result<()> {
        let mut current_data = data;
        loop {
            use indicatif::{ProgressBar, ProgressStyle};
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
            // ... (rest of the code)

            if let Some(t) = thought {
                if !t.trim().is_empty() {
                    ui::print_rule(Some("Thought"), Some("bright_black"));
                    ui::print_block(&t, None, Some("bright_black"));
                    ui::print_rule(None, Some("bright_black"));
                }
            }

            if let Some(text) = response {
                if !text.trim().is_empty() {
                    ui::print_block(&text, Some(&self.client.get_display_name()), Some("cyan"));
                }
            }

            // Handle incoming images in the response
            let last_msg = self.client.get_state().conversation.last().cloned();
            if let Some(msg) = last_msg {
                if msg.role == Role::Assistant || msg.role == Role::Model {
                    for part in &msg.parts {
                        if let MessagePart::Part(cp) = part {
                            if let Some(id) = &cp.inline_data {
                                let b64_data =
                                    id.get("data").and_then(|v| v.as_str()).unwrap_or("");
                                let mime_type =
                                    id.get("mimeType").and_then(|v| v.as_str()).unwrap_or("");
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
                                        Err(e) => ui::report_error(&format!(
                                            "Failed to save image: {}",
                                            e
                                        )),
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Handle tool calls
            let mut tool_results = Vec::new();
            let last_msg = self.client.get_state().conversation.last().cloned();

            if let Some(msg) = last_msg {
                if msg.role == Role::Assistant || msg.role == Role::Model {
                    for part in &msg.parts {
                        if let MessagePart::Part(cp) = part {
                            if let Some(fc) = &cp.function_call {
                                let name = fc.get("name").and_then(|v| v.as_str()).unwrap_or("");
                                let args = fc
                                    .get("arguments")
                                    .and_then(|v| v.as_object())
                                    .cloned()
                                    .unwrap_or_default();
                                let id = fc.get("id").and_then(|v| v.as_str()).unwrap_or("");

                                ui::print_tool_call(name, &serde_json::json!(args));

                                // Dual LLM Verification (Start)
                                let mut verifier_handle = None;
                                let config = crate::config::CONFIG_MANAGER.get_config();
                                if config.security.dual_llm_verification.unwrap_or(false) {
                                    // 1. Extract recent USER messages (Sliding Window: last 5)
                                    // 2. Truncate long messages to focus on intent, not data (UTF-8 safe)
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

                                    // 3. Final total length safety check
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

                                    verifier_handle = Some(tokio::spawn(async move {
                                        crate::security::dual_llm_verifier::verify_tool_call(
                                            &intent_context,
                                            &name_clone,
                                            &args_clone,
                                            None,
                                        )
                                        .await
                                    }));
                                }

                                // HITL Approval
                                let risk_level =
                                    crate::security::cass::CASS_ORCHESTRATOR.evaluate_risk(name);
                                let auto_approval = config
                                    .security
                                    .auto_approval_level
                                    .as_deref()
                                    .unwrap_or("none");

                                let mut approved = false;
                                if auto_approval == "low"
                                    && risk_level == crate::security::cass::RiskLevel::Low
                                {
                                    approved = true;
                                    ui::report_success("Auto-approved (Low Risk)");
                                } else if auto_approval == "medium"
                                    && (risk_level == crate::security::cass::RiskLevel::Low
                                        || risk_level == crate::security::cass::RiskLevel::Medium)
                                {
                                    approved = true;
                                    ui::report_success("Auto-approved (Medium Risk)");
                                }

                                if !approved {
                                    let prompt = format!("Execute {}", name);
                                    approved = ui::ask_confirm(&prompt);
                                }

                                let mut final_result = None;

                                if !approved {
                                    ui::report_warning("Execution cancelled by user.");
                                    let feedback =
                                        ui::get_user_input("Provide feedback (optional): ");
                                    let result_msg = if feedback.trim().is_empty() {
                                        "Error: Execution cancelled by user.".to_string()
                                    } else {
                                        format!(
                                            "Error: Execution cancelled by user. Feedback: {}",
                                            feedback
                                        )
                                    };
                                    final_result = Some(serde_json::Value::String(result_msg));
                                }

                                // Wait for Dual LLM Verifier if it was started
                                if final_result.is_none() {
                                    if let Some(handle) = verifier_handle {
                                        let pb = ProgressBar::new_spinner();
                                        pb.set_style(
                                            ProgressStyle::default_spinner()
                                                .template("{spinner:.yellow} {msg}")?,
                                        );

                                        let config = crate::config::CONFIG_MANAGER.get_config();
                                        let v_provider = config.security.dual_llm_provider.clone();
                                        let v_model_alias = config.security.dual_llm_model.clone();
                                        let v_model_name = {
                                            let registry = crate::llm::registry::CLIENT_REGISTRY
                                                .lock()
                                                .unwrap();
                                            registry
                                                .create_client(
                                                    &v_provider,
                                                    &v_model_alias,
                                                    false,
                                                    true,
                                                )
                                                .map(|c| c.get_state().model.clone())
                                                .unwrap_or(v_model_alias)
                                        };

                                        pb.set_message(format!(
                                            "Finalizing intent verification... ({})",
                                            v_model_name
                                        ));
                                        pb.enable_steady_tick(std::time::Duration::from_millis(
                                            100,
                                        ));

                                        let (safe, reason) = handle.await.unwrap_or_else(|_| {
                                            (false, "Verification task panicked".to_string())
                                        });
                                        pb.finish_and_clear();
                                        if !safe {
                                            ui::report_error(&format!(
                                                "Dual LLM Verification failed: {}",
                                                reason
                                            ));
                                            final_result = Some(serde_json::Value::String(
                                                format!("Security Policy Violation: {}", reason),
                                            ));
                                        } else {
                                            ui::report_success(&format!(
                                                "Intent Verified: {}",
                                                reason
                                            ));
                                        }
                                    }
                                }

                                // Execute tool
                                // 1. Static Analysis (Space)
                                if final_result.is_none()
                                    && (name == "execute_command" || name == "execute_python")
                                {
                                    let mut check_contents = Vec::new();
                                    if let Some(c) = args.get("command").and_then(|v| v.as_str()) {
                                        check_contents.push(c.to_string());
                                    }
                                    if let Some(c) = args.get("code").and_then(|v| v.as_str()) {
                                        check_contents.push(c.to_string());
                                    }
                                    if let Some(serde_json::Value::Array(arr)) = args.get("args") {
                                        for v in arr {
                                            if let Some(s) = v.as_str() {
                                                check_contents.push(s.to_string());
                                            }
                                        }
                                    }

                                    for code in check_contents {
                                        let (safe, violations, warnings) =
                                            crate::security::static_analyzer::StaticAnalyzer::analyze_python_safety(
                                                &code,
                                            );
                                        if !safe {
                                            let err = format!(
                                                "Static Analysis Blocked: {}",
                                                violations.join(", ")
                                            );
                                            ui::report_error(&err);
                                            final_result = Some(serde_json::Value::String(err));
                                            break;
                                        } else if !warnings.is_empty() {
                                            ui::report_warning(&format!(
                                                "Static Analysis Warning: {}",
                                                warnings.join(", ")
                                            ));
                                        }
                                    }
                                }

                                let result_value = if let Some(res) = final_result {
                                    res
                                } else {
                                    let result =
                                        self.execute_tool(name, args.clone().into_iter().collect());
                                    match result {
                                        Ok(v) => {
                                            let audit_ctx = serde_json::json!({
                                                "trace_id": id,
                                                "model": self.client.get_state().model,
                                                "user_id": "current_user"
                                            });
                                            crate::security::audit::log_audit(
                                                name,
                                                serde_json::json!(args),
                                                v.as_str(),
                                                Some(0),
                                                None,
                                                Some(&audit_ctx),
                                            );
                                            v
                                        }
                                        Err(e) => {
                                            let audit_ctx = serde_json::json!({
                                                "trace_id": id,
                                                "model": self.client.get_state().model,
                                                "user_id": "current_user"
                                            });
                                            crate::security::audit::log_audit(
                                                name,
                                                serde_json::json!(args),
                                                None,
                                                Some(1),
                                                Some(&e.to_string()),
                                                Some(&audit_ctx),
                                            );
                                            serde_json::Value::String(format!("Error: {}", e))
                                        }
                                    }
                                };

                                let result_str = if let Some(s) = result_value.as_str() {
                                    s.to_string()
                                } else {
                                    result_value.to_string()
                                };

                                ui::print_tool_result(&result_str);

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
                }
            }

            if tool_results.is_empty() {
                break;
            } else {
                self.client.get_state_mut().conversation.push(Message {
                    role: Role::Tool,
                    parts: tool_results,
                });
            }
        }

        Ok(())
    }

    fn execute_tool(
        &self,
        name: &str,
        args: HashMap<String, serde_json::Value>,
    ) -> anyhow::Result<serde_json::Value> {
        let registry = crate::tools::registry::REGISTRY.lock().unwrap();
        if let Some(tool) = registry.tools.get(name) {
            (tool.func)(args)
        } else {
            Err(anyhow::anyhow!("Tool not found: {}", name))
        }
    }

    fn handle_interruption(&mut self) {
        let state = self.client.get_state_mut();
        let last_msg = state.conversation.last().cloned();
        if let Some(msg) = last_msg {
            if msg.role == Role::Assistant || msg.role == Role::Model {
                let mut has_unanswered_tools = false;
                for part in &msg.parts {
                    if let MessagePart::Part(cp) = part {
                        if cp.function_call.is_some() {
                            has_unanswered_tools = true;
                            break;
                        }
                    }
                }

                if has_unanswered_tools {
                    let mut tool_results = Vec::new();
                    for part in &msg.parts {
                        if let MessagePart::Part(cp) = part {
                            if let Some(fc) = &cp.function_call {
                                let name = fc.get("name").and_then(|v| v.as_str()).unwrap_or("");
                                let id = fc.get("id").and_then(|v| v.as_str()).unwrap_or("");

                                let mut fr = HashMap::new();
                                fr.insert("id".to_string(), serde_json::json!(id));
                                fr.insert("name".to_string(), serde_json::json!(name));
                                fr.insert(
                                    "response".to_string(),
                                    serde_json::json!("Error: Interrupted by user."),
                                );

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
                    state.conversation.push(Message {
                        role: Role::Tool,
                        parts: tool_results,
                    });
                }
            }
        }
    }
}
