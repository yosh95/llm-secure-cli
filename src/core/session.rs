use crate::cli::interactive::completion::ChatCompleter;
use crate::cli::ui;
use crate::consts::HISTORY_LOG_PATH;
use crate::llm::base::LlmClient;
use crate::llm::models::{ContentPart, DataSource, Message, MessagePart, Role};
use crate::security::merkle_anchor::SessionAnchorManager;
use crate::security::runtime::{get_runtime, SecureRuntime, Task, TaskStatus};
use colored::*;
use rustyline::error::ReadlineError;
use rustyline::history::FileHistory;
use rustyline::{Cmd, Editor, KeyCode, KeyEvent, Modifiers};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub struct ChatSession {
    pub client: Box<dyn LlmClient>,
    pub intent: String,
    pub pending_data: Vec<DataSource>,
    pub pending_tasks: Vec<Task>,
    pub runtime: Box<dyn SecureRuntime>,
    pub trace_id: String,
}

impl Drop for ChatSession {
    fn drop(&mut self) {
        // Automatically create a PQC-signed session anchor when the session object is destroyed.
        // This ensures the session appears in 'llsc identity list-sessions'.
        let _ = SessionAnchorManager::create_anchor(&self.trace_id);
    }
}

impl ChatSession {
    pub fn new(client: Box<dyn LlmClient>) -> Self {
        let config = crate::config::CONFIG_MANAGER.get_config();
        let runtime = get_runtime(&config.security.runtime_type);
        let trace_id = format!("sess-{}", &uuid::Uuid::new_v4().to_string()[..8]);

        // Log session initialization for anchoring
        crate::security::audit::log_audit(
            "session_start",
            "session",
            serde_json::json!({}),
            None,
            None,
            None,
            Some(&serde_json::json!({
                "trace_id": trace_id,
                "model": client.get_state().model,
                "user_id": "current_user"
            })),
        );

        Self {
            client,
            intent: String::new(),
            pending_data: Vec::new(),
            pending_tasks: Vec::new(),
            runtime,
            trace_id,
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

    pub async fn execute_pending_tasks(&mut self) -> anyhow::Result<()> {
        if self.pending_tasks.is_empty() {
            ui::report_warning("No pending tasks to execute.");
            return Ok(());
        }

        ui::report_info(&format!(
            "Executing {} pending tasks using {} runtime...",
            self.pending_tasks.len(),
            self.runtime.name()
        ));

        let tasks = std::mem::take(&mut self.pending_tasks);
        let mut results = Vec::new();

        for mut task in tasks {
            ui::report_info(&format!("Running task: {} ({})", task.description, task.id));
            task.status = TaskStatus::Running;

            match self.runtime.execute_task(task.clone()).await {
                Ok(res) => {
                    ui::report_success(&format!("Task {} completed.", task.id));
                    task.status = TaskStatus::Completed;
                    results.push(format!("Task {}: {}", task.id, res));
                }
                Err(e) => {
                    ui::report_error(&format!("Task {} failed: {}", task.id, e));
                    task.status = TaskStatus::Failed(e.to_string());
                    results.push(format!("Task {} failed: {}", task.id, e));
                }
            }
        }

        if !results.is_empty() {
            let result_msg = format!("Execution Results:\n{}", results.join("\n"));
            self.pending_data.push(DataSource {
                content: serde_json::Value::String(result_msg),
                content_type: "text/plain".to_string(),
                is_file_or_url: false,
                metadata: std::collections::HashMap::new(),
            });
            ui::report_info("Results added to pending data for next turn.");
        }

        Ok(())
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

        println!("{}", "Use Ctrl+C or /q to exit, /h for help.".dimmed());

        let current_provider = Arc::new(Mutex::new(self.client.get_state().provider.clone()));

        // Configure rustyline for a more "rich" experience
        let config = rustyline::Config::builder()
            .history_ignore_space(true)
            .completion_type(rustyline::CompletionType::List)
            .edit_mode(rustyline::EditMode::Emacs)
            .bracketed_paste(true)
            .build();

        let mut rl = Editor::<ChatCompleter, FileHistory>::with_config(config)
            .expect("Failed to create editor");
        rl.set_helper(Some(ChatCompleter::new(current_provider.clone())));

        // Bindings to improve multi-line and history navigation
        // Up/Down now jump between history entries even if they are multi-line
        rl.bind_sequence(KeyEvent(KeyCode::Up, Modifiers::NONE), Cmd::PreviousHistory);
        rl.bind_sequence(KeyEvent(KeyCode::Down, Modifiers::NONE), Cmd::NextHistory);
        // Ctrl-J for inserting a new line without submitting
        rl.bind_sequence(KeyEvent(KeyCode::Char('j'), Modifiers::CTRL), Cmd::Newline);
        // Ctrl-Up/Down for navigating between lines within a multi-line entry
        rl.bind_sequence(KeyEvent(KeyCode::Up, Modifiers::CTRL), Cmd::PreviousHistory);
        rl.bind_sequence(KeyEvent(KeyCode::Down, Modifiers::CTRL), Cmd::NextHistory);

        if let Some(parent) = HISTORY_LOG_PATH.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = rl.load_history(&*HISTORY_LOG_PATH);

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
                rl.readline("> ")
            };

            match readline {
                Ok(line) => {
                    let final_trimmed = line.trim().to_string();
                    if final_trimmed.is_empty() {
                        continue;
                    }

                    // Handle commands
                    let (content, should_continue) =
                        match crate::cli::interactive::dispatcher::handle_command(
                            self,
                            &final_trimmed,
                        )
                        .await
                        {
                            crate::cli::interactive::dispatcher::CommandResult::Exit => break,
                            crate::cli::interactive::dispatcher::CommandResult::Handled => {
                                let _ = rl.add_history_entry(&final_trimmed);
                                (None, true)
                            }
                            crate::cli::interactive::dispatcher::CommandResult::NotACommand => {
                                (Some(final_trimmed.clone()), false)
                            }
                            crate::cli::interactive::dispatcher::CommandResult::Input(text) => {
                                next_initial_text = Some(text);
                                (None, true)
                            }
                        };

                    if should_continue {
                        continue;
                    }

                    let _ = rl.add_history_entry(&final_trimmed);
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

                                let mut final_result = None;

                                // --- [PHASE 1] Lightweight Fast Checks (Block Early) ---

                                // 1.1 Path Guardrails
                                let path_args =
                                    ["path", "directory", "file", "src", "dest", "filename"];
                                for arg_name in path_args {
                                    if let Some(p_val) = args.get(arg_name).and_then(|v| v.as_str())
                                    {
                                        if let Err(e) =
                                            crate::security::path_validator::validate_path(p_val)
                                        {
                                            let err_msg = format!(
                                                "Security Blocked (Path Guardrails): {}",
                                                e
                                            );
                                            ui::report_error(&err_msg);
                                            final_result = Some(serde_json::Value::String(err_msg));
                                            break;
                                        }
                                    }
                                }

                                // 1.2 Static Analysis (for execute_command)
                                if final_result.is_none() && name == "execute_command" {
                                    let program =
                                        args.get("command").and_then(|v| v.as_str()).unwrap_or("");
                                    let cmd_args: Vec<String> = args
                                        .get("args")
                                        .and_then(|v| v.as_array())
                                        .map(|arr| {
                                            arr.iter()
                                                .filter_map(|v| v.as_str())
                                                .map(|s| s.to_string())
                                                .collect()
                                        })
                                        .unwrap_or_default();

                                    let (safe, violations) =
                                        crate::security::static_analyzer::StaticAnalyzer::check(
                                            program, &cmd_args,
                                        );
                                    if !safe {
                                        let err_msg = format!(
                                            "Security Blocked (Static Analysis): {}",
                                            violations.join(", ")
                                        );
                                        ui::report_error(&err_msg);
                                        final_result = Some(serde_json::Value::String(err_msg));
                                    }
                                }

                                // 1.3 ABAC Policy Engine (Externalized Rules)
                                if final_result.is_none() {
                                    let mut eval_ctx =
                                        crate::security::policy::EvaluationContext::new();
                                    eval_ctx.set_attribute(
                                        "tool",
                                        serde_json::Value::String(name.to_string()),
                                    );

                                    if !crate::security::policy::POLICY_ENGINE
                                        .evaluate(name, &args, &eval_ctx)
                                    {
                                        let err_msg = format!(
                                            "Security Blocked (ABAC Policy): Execution denied for tool '{}'",
                                            name
                                        );
                                        ui::report_error(&err_msg);
                                        final_result = Some(serde_json::Value::String(err_msg));
                                    }
                                }

                                // --- [PHASE 2] High-Assurance Checks (Dual LLM & HITL) ---

                                if final_result.is_none() {
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
                                                    let head: String =
                                                        text.chars().take(500).collect();
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
                                    let risk_level = crate::security::cass::CASS_ORCHESTRATOR
                                        .evaluate_risk(name);
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
                                            || risk_level
                                                == crate::security::cass::RiskLevel::Medium)
                                    {
                                        approved = true;
                                        ui::report_success("Auto-approved (Medium Risk)");
                                    }

                                    if !approved {
                                        let prompt = format!("Execute {}", name);
                                        approved = ui::ask_confirm(&prompt);
                                    }

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
                                            let v_provider =
                                                config.security.dual_llm_provider.clone();
                                            let v_model_alias =
                                                config.security.dual_llm_model.clone();
                                            let v_model_name = {
                                                let registry =
                                                    crate::llm::registry::CLIENT_REGISTRY
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
                                            pb.enable_steady_tick(
                                                std::time::Duration::from_millis(100),
                                            );

                                            let (safe, reason) =
                                                handle.await.unwrap_or_else(|_| {
                                                    (
                                                        false,
                                                        "Verification task panicked".to_string(),
                                                    )
                                                });
                                            pb.finish_and_clear();
                                            if !safe {
                                                ui::report_error(&format!(
                                                    "Dual LLM Verification failed: {}",
                                                    reason
                                                ));
                                                final_result =
                                                    Some(serde_json::Value::String(format!(
                                                        "Security Policy Violation: {}",
                                                        reason
                                                    )));
                                            } else {
                                                ui::report_success(&format!(
                                                    "Intent Verified: {}",
                                                    reason
                                                ));
                                            }
                                        }
                                    }
                                }

                                // --- [PHASE 3] Execution ---
                                let result_value = if let Some(res) = final_result {
                                    res
                                } else {
                                    let result =
                                        self.execute_tool(name, args.clone().into_iter().collect());
                                    match result {
                                        Ok(v) => {
                                            let audit_ctx = serde_json::json!({
                                                "trace_id": self.trace_id,
                                                "model": self.client.get_state().model,
                                                "user_id": "current_user"
                                            });
                                            crate::security::audit::log_audit(
                                                "tool_call",
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
                                                "trace_id": self.trace_id,
                                                "model": self.client.get_state().model,
                                                "user_id": "current_user"
                                            });
                                            crate::security::audit::log_audit(
                                                "tool_call",
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
        &mut self,
        name: &str,
        args: HashMap<String, serde_json::Value>,
    ) -> anyhow::Result<serde_json::Value> {
        let registry = crate::tools::registry::REGISTRY.lock().unwrap();
        if let Some(tool) = registry.tools.get(name) {
            let res = (tool.func)(args.clone());

            if name == "queue_task" && res.is_ok() {
                let task = Task {
                    id: uuid::Uuid::new_v4().to_string(),
                    task_type: args
                        .get("task_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string(),
                    description: args
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("No description")
                        .to_string(),
                    parameters: args
                        .get("parameters")
                        .cloned()
                        .unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
                    status: TaskStatus::Pending,
                };
                self.pending_tasks.push(task);
            }

            res
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
