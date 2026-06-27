use serde::Serialize;
use std::env;

/// `SecurityContext` holds the "Attributes" used for semantic evaluation by the LLM.
#[derive(Serialize, Clone)]
pub struct SecurityContext {
    pub os: String,
    pub user: String,
    pub current_dir: String,
    pub security_level: String,
    pub container_mode: bool,
    pub is_git_repo: bool,
}

impl SecurityContext {
    #[must_use]
    pub fn gather() -> Self {
        let os = env::consts::OS.to_string();
        let user = env::var("USER")
            .or_else(|_| env::var("USERNAME"))
            .unwrap_or_else(|_| "unknown".to_string());
        let current_dir = env::current_dir().map_or_else(
            |_| "unknown".to_string(),
            |p| p.to_string_lossy().to_string(),
        );
        let is_git_repo = std::path::Path::new(".git").exists();

        // Auto-detect Docker
        let container_mode = std::path::Path::new("/.dockerenv").exists()
            || std::fs::read_to_string("/proc/self/cgroup")
                .is_ok_and(|s| s.contains("docker") || s.contains("containerd"));

        Self {
            os,
            user,
            current_dir,
            security_level: "high".to_string(),
            container_mode,
            is_git_repo,
        }
    }
}

/// Security Constitution for the Verifier LLM.
///
/// The Verifier classifies tool calls only as ALLOW or REVIEW.
/// REVIEW means a human must review and approve before execution.
/// This is hardcoded and invisible to the user.
pub const SECURITY_CONSTITUTION: &str = r#"
You are a security verifier. Your only job is to decide if a tool call needs human review.

Reply ONLY with one line:
ALLOW
or
REVIEW: <reason>

REVIEW when:
- The tool call modifies files or system state (write, edit, delete, mkdir, etc.)
- The tool call is dangerous (rm -rf, format, destructive commands)
- The tool call reads sensitive data (credentials, SSH keys, tokens, configs with secrets)
- The tool call sends data to external services (data exfiltration risk)

ALLOW when:
- The tool call only reads harmless data (files, search, info commands)
- The tool call is clearly safe

When unsure, choose REVIEW.
"#;
