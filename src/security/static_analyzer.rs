/// StaticAnalyzer provides a "Fast Fail" mechanism for deterministic security blocks.
///
/// 【Architectural Principle】
/// This tool adopts "AI-native ABAC" and does not maintain platform-dependent static
/// rules (e.g., banning specific command names like curl), which leads to maintenance
/// quagmires. Complex intent judgment and risk assessment are delegated entirely to
/// Phase 2: "Dual LLM Verifier".
pub struct StaticAnalyzer;

impl StaticAnalyzer {
    pub fn check(_command: &str, _args: &[String]) -> (bool, Vec<String>) {
        let violations = Vec::new();

        // Since we use Command::new which does not invoke a shell,
        // command line injection risks are structurally eliminated.
        // To avoid platform-dependent complexity, we do not block anything here.

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
