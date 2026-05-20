use crate::core::session::ActiveSession;
use crate::security::cass::RiskLevel;
use serde_json::Value;
use std::collections::HashMap;

impl ActiveSession {
    /// Phase 4: Tool execution with audit logging and result display.
    pub(crate) async fn phase4_execute_and_audit(
        &mut self,
        name: &str,
        args: &serde_json::Map<String, Value>,
        risk_level: RiskLevel,
        approved: bool,
    ) -> Value {
        self.execute_and_audit_tool(name, args, risk_level, approved)
            .await
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

        let mut is_error = false;
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
                is_error = true;
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

        // Display result if it was not auto-approved OR if an error occurred.
        // This ensures that auto-approved successful calls don't clutter the UI with output (like stdout),
        // but failures are always shown. The tool call itself is always printed above.
        if !approved || is_error {
            let display_str = final_v
                .as_str()
                .map(|s| s.to_owned())
                .unwrap_or_else(|| final_v.to_string());
            self.ctx.ui.print_tool_result(&display_str);

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
}
