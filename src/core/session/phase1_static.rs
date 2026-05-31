use crate::core::session::ActiveSession;
use serde_json::Value;

impl ActiveSession {
    /// Phase 1: Static analysis — deterministic fast-fail for null bytes and control characters.
    pub(crate) fn phase1_static_check(
        &self,
        name: &str,
        args: &serde_json::Map<String, Value>,
        config: &crate::config::models::AppConfig,
    ) -> anyhow::Result<()> {
        if let Err(e) = crate::security::validate_tool_call(name, args, &config.security) {
            self.ctx.ui.report_error(&e);
            return Err(anyhow::anyhow!("Phase 1 blocked: {e}"));
        }
        Ok(())
    }
}
