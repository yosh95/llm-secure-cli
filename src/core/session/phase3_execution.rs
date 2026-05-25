use crate::core::session::ActiveSession;
use serde_json::Value;
use std::collections::HashMap;

impl ActiveSession {
    /// Phase 4: Tool execution with audit logging and result display.
    pub(crate) async fn phase3_execute_and_audit(
        &mut self,
        name: &str,
        args: &serde_json::Map<String, Value>,
        approved: bool,
    ) -> Value {
        self.execute_and_audit_tool(name, args, approved).await
    }

    /// Internal execution and audit logging logic.
    async fn execute_and_audit_tool(
        &mut self,
        name: &str,
        args: &serde_json::Map<String, Value>,
        _approved: bool,
    ) -> Value {
        let config = match self.ctx.config_manager.get_config() {
            Ok(c) => c,
            Err(e) => return Value::String(format!("Error: Failed to load config: {}", e)),
        };
        let user_id = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());

        let args_map: HashMap<String, Value> =
            args.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        let result = tokio::select! {
            res = self.execute_tool(name, &args_map) => res,
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

        let name_owned = name.to_string();
        let mut final_v = match result {
            Ok(v) => {
                let c = config.clone();
                let a = args.clone();
                let ac = audit_ctx.clone();
                let out_str = v.as_str().map(|s| s.to_string());
                let entry = tokio::task::spawn_blocking(move || {
                    crate::security::audit::log_audit_and_return(
                        crate::security::audit::AuditParams {
                            event_type: "tool_call",
                            tool_name: &name_owned,
                            args: serde_json::json!(a),
                            output: out_str.as_deref(),
                            exit_code: Some(0),
                            error: None,
                            context: Some(&ac),
                            config: &c,
                        },
                        None,
                    )
                })
                .await;
                if let Ok(Some(entry)) = entry {
                    self.audit_entries.push(entry);
                }
                v
            }
            Err(e) => {
                let err_msg = e.to_string();
                let err_msg_for_closure = err_msg.clone();
                let c = config.clone();
                let a = args.clone();
                let ac = audit_ctx.clone();
                let entry = tokio::task::spawn_blocking(move || {
                    crate::security::audit::log_audit_and_return(
                        crate::security::audit::AuditParams {
                            event_type: "tool_call",
                            tool_name: &name_owned,
                            args: serde_json::json!(a),
                            output: None,
                            exit_code: Some(1),
                            error: Some(&err_msg_for_closure),
                            context: Some(&ac),
                            config: &c,
                        },
                        None,
                    )
                })
                .await;
                if let Ok(Some(entry)) = entry {
                    self.audit_entries.push(entry);
                }
                Value::String(format!("Error: {}", err_msg))
            }
        };

        crate::tools::executor_utils::truncate_json_strings(
            &mut final_v,
            config.general.max_output_lines,
            config.general.max_output_chars,
        );

        // Calculate and display stats
        let stats = crate::cli::stats::get_tool_result_stats(name, &final_v);

        // Display the tool result to the user.
        // - Auto-approved (verifier-signed): skip stdout display, but show errors/stderr.
        // - Manual approval (HITL): show everything as before.
        let display_str = final_v
            .as_str()
            .map(|s| s.to_owned())
            .unwrap_or_else(|| final_v.to_string());

        if _approved {
            // Auto-approved: skip stdout display, only show stderr (errors/warnings).
            // For structured results (JSON with stdout/stderr), extract stderr.
            // For plain string results, show only if it starts with "Error:" or contains warnings.
            if let Some(obj) = final_v.as_object() {
                let stderr = obj
                    .get("stderr")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let exit_code = obj.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(0);
                if !stderr.is_empty() || exit_code != 0 {
                    // Show error output only
                    let mut err_out = String::from(
                        "  ── Result ──
",
                    );
                    if !stderr.is_empty() {
                        err_out.push_str(
                            "STDERR:
",
                        );
                        err_out.push_str(stderr);
                        if !stderr.ends_with('\n') {
                            err_out.push('\n');
                        }
                    }
                    if exit_code != 0 {
                        err_out.push_str(&format!(
                            "Exit Code: {}
",
                            exit_code
                        ));
                    }
                    self.ctx.ui.print_tool_result(&err_out);
                }
                // else: skip display entirely for clean auto-approved calls
            } else if display_str.starts_with("Error:") {
                self.ctx.ui.print_tool_result(&display_str);
            }
            // else: pure success, no display (stdout would have been shown on demand)
        } else {
            // Manual approval: show everything as before
            self.ctx.ui.print_tool_result(&display_str);
        }

        // Show stats (stderr info included)
        crate::cli::stats::print_tool_stats(&stats);

        // Convert to human-readable string for the LLM
        let human_result = crate::tools::executor_utils::humanize_tool_result(name, &final_v);
        Value::String(human_result)
    }
}
