use crate::core::session::ActiveSession;
use serde_json::Value;
use std::collections::HashMap;

impl ActiveSession {
    /// Phase 3: Tool execution with audit logging and result display.
    pub(crate) fn phase3_execute_and_audit(
        &mut self,
        name: &str,
        args: &serde_json::Map<String, Value>,
        approved: bool,
    ) -> Value {
        self.execute_and_audit_tool(name, args, approved)
    }

    /// Internal execution and audit logging logic.
    fn execute_and_audit_tool(
        &mut self,
        name: &str,
        args: &serde_json::Map<String, Value>,
        _approved: bool,
    ) -> Value {
        let config = match self.ctx.config_manager.get_config() {
            Ok(c) => c,
            Err(e) => return Value::String(format!("Error: Failed to load config: {e}")),
        };
        let user_id = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());

        let args_map: HashMap<String, Value> =
            args.iter().map(|(k, v)| (k.clone(), v.clone())).collect();

        let is_stdout = self.client.get_state().stdout;

        let result = self.execute_tool(name, &args_map);

        let audit_ctx = serde_json::json!({
            "trace_id": self.trace_id,
            "model": self.client.get_state().model.clone(),
            "provider": self.client.get_state().provider.clone(),
            "user_id": user_id
        });

        let mut final_v = match result {
            Ok(v) => {
                let out_str = v.as_str().map(std::string::ToString::to_string);
                let entry = crate::security::audit::log_audit_and_return(
                    crate::security::audit::AuditParams {
                        event_type: "tool_call",
                        tool_name: name,
                        args: serde_json::json!(args),
                        output: out_str.as_deref(),
                        exit_code: Some(0),
                        error: None,
                        context: Some(&audit_ctx),
                        config: &config,
                    },
                    None,
                );
                if let Some(entry) = entry {
                    self.audit_entries.push(entry);
                }
                v
            }
            Err(e) => {
                let err_msg = e.to_string();
                let entry = crate::security::audit::log_audit_and_return(
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
                );
                if let Some(entry) = entry {
                    self.audit_entries.push(entry);
                }
                Value::String(format!("Error: {err_msg}"))
            }
        };

        crate::tools::executor_utils::truncate_json_strings(
            &mut final_v,
            config.general.max_output_lines,
            config.general.max_output_chars,
        );

        // Display the tool result to the user.
        // The result is always sent to the LLM and logged to the audit trail.
        if !is_stdout {
            let display_str = final_v
                .as_str()
                .map_or_else(|| final_v.to_string(), std::borrow::ToOwned::to_owned);

            self.ctx.ui.print_tool_result(&display_str);
        }

        // Convert to human-readable string for the LLM
        let human_result = crate::tools::executor_utils::humanize_tool_result(name, &final_v);
        Value::String(human_result)
    }
}
