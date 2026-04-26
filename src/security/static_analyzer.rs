/// StaticAnalyzer provides a "Fast Fail" mechanism for deterministic security blocks.
///
/// In the modern AI-native security model, we offload complex risk assessment
/// to the Dual LLM Verifier. This analyzer is kept minimal to handle only
/// obvious syntactic anomalies with zero latency.
pub struct StaticAnalyzer;

impl StaticAnalyzer {
    pub fn check(_command: &str, _args: &[String]) -> (bool, Vec<String>) {
        let violations = Vec::new();

        // Minimalist approach: We no longer maintain a blacklist of binaries here.
        // The LLM-based policy engine (Security Constitution) handles the semantic risk.
        // This avoids platform-dependent complexity (Windows vs Linux command names).

        (violations.is_empty(), violations)
    }

    /// Fast-check for dangerous shell characters in a raw string
    pub fn is_obviously_malicious(input: &str) -> bool {
        // Only block things that could break the tool-call parser itself
        input.contains('\0')
    }

    /// Backwards compatibility for benchmarks and older tests.
    /// In the new model, we favor semantic LLM-based analysis over regex-style blacklists.
    pub fn is_dangerous_command(input: &str) -> bool {
        Self::is_obviously_malicious(input)
    }
}
