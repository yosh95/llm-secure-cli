/// StaticAnalyzer provides a "Fast Fail" mechanism for deterministic security blocks.
///
/// 【Architectural Principle】
/// This tool adopts "AI-native ABAC" and does not maintain platform-dependent static
/// rules (e.g., banning specific command names like curl), which leads to maintenance
/// quagmires. Complex intent judgment and risk assessment are delegated entirely to
/// Phase 2: "Dual LLM Verifier".
pub struct StaticAnalyzer;

impl StaticAnalyzer {
    pub fn check(command: &str, args: &[String]) -> (bool, Vec<String>) {
        let mut violations = Vec::new();

        // Block explicit shell invocation that would bypass structural safety.
        // Command::new does not invoke a shell, but `sh -c "..."` or `bash -c "..."`
        // would re-introduce shell injection risks — the exact vulnerability our
        // architecture is designed to structurally eliminate.
        let shell_names = ["sh", "bash", "zsh", "fish", "dash", "ksh", "csh", "tcsh"];
        if shell_names.contains(&command)
            && args.iter().any(|a| a == "-c" || a == "-e" || a == "-i")
        {
            violations.push(
                "Shell invocation with -c flag is blocked. This bypasses the structural \
                     safety of Command::new (no-shell). Use built-in tools (grep_files, \
                     search_files, etc.) or execute commands directly without a shell."
                    .to_string(),
            );
        }

        // Block obviously malicious input in command name
        if Self::is_obviously_malicious(command) {
            violations.push(
                "Command contains control characters or null bytes: blocked for safety."
                    .to_string(),
            );
        }

        // Block obviously malicious input in any argument
        for arg in args {
            if Self::is_obviously_malicious(arg) {
                violations.push(
                    "Argument contains control characters or null bytes: blocked for safety."
                        .to_string(),
                );
                break;
            }
        }

        (violations.is_empty(), violations)
    }

    /// Block only physical anomalies that could disrupt the tool execution engine
    /// or log output.
    pub fn is_obviously_malicious(input: &str) -> bool {
        // Block NULL bytes and control characters except for newline, carriage return, and tab.
        // These can cause unstable behavior in OS or terminals.
        input
            .chars()
            .any(|c| c == '\0' || (c.is_control() && c != '\n' && c != '\r' && c != '\t'))
    }

    /// Backwards compatibility for benchmarks and older tests.
    pub fn is_dangerous_command(input: &str) -> bool {
        Self::is_obviously_malicious(input)
    }
}
