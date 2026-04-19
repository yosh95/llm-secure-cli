pub struct StaticAnalyzer;

impl StaticAnalyzer {
    pub fn is_dangerous_command(command: &str) -> (bool, Vec<String>) {
        let mut violations = Vec::new();

        let dangerous_patterns = [
            "rm -rf /",
            "mkfs",
            "dd if=",
            "> /etc/",
            "chmod -R 777",
            "chown",
            "passwd",
        ];

        for pattern in dangerous_patterns {
            if command.contains(pattern) {
                violations.push(format!("Forbidden command pattern: {}", pattern));
            }
        }

        (violations.is_empty(), violations)
    }
}
