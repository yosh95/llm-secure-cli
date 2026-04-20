use once_cell::sync::Lazy;
use regex::Regex;

pub struct StaticAnalyzer;

#[derive(Debug)]
pub struct Violation {
    pub description: &'static str,
    pub pattern: String,
}

static DANGEROUS_BINARIES: &[&str] = &[
    "mkfs", "fdisk", "parted", "dd", "passwd", "chown", "chmod", "kill", "reboot", "shutdown",
];

static SENSITIVE_PATHS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        Regex::new(r"^/etc/.*").unwrap(),
        Regex::new(r"^/root/.*").unwrap(),
        Regex::new(r"^/var/.*").unwrap(),
        Regex::new(r"^/proc/.*").unwrap(),
        Regex::new(r"^/sys/.*").unwrap(),
        Regex::new(r"^/dev/.*").unwrap(),
    ]
});

impl StaticAnalyzer {
    pub fn check(command: &str, args: &[String]) -> (bool, Vec<String>) {
        let mut violations = Vec::new();

        // 1. Binary blocklist
        if DANGEROUS_BINARIES.contains(&command) {
            violations.push(format!("Use of forbidden binary: {}", command));
        }

        // 2. Argument-based checks
        match command {
            "rm" if args
                .iter()
                .any(|arg| arg == "/" || arg == "/*" || arg.starts_with("/etc")) =>
            {
                violations.push("Destructive removal of sensitive directory".to_string());
            }
            "curl" | "wget"
                if args
                    .iter()
                    .any(|arg| arg.contains("sh") && arg.contains("|")) =>
            {
                // Check for piping to shell (not directly possible via argv, but check for suspicious URLs)
                violations.push("Potential remote script execution".to_string());
            }
            "find" if args.iter().any(|arg| arg == "-exec" || arg == "-delete") => {
                violations.push("Forbidden find flags (-exec, -delete)".to_string());
            }
            _ => {}
        }

        // 3. Sensitive path injection in any argument
        for arg in args {
            for path_regex in SENSITIVE_PATHS.iter() {
                if path_regex.is_match(arg) {
                    violations.push(format!("Access to sensitive path in arguments: {}", arg));
                }
            }
        }

        (violations.is_empty(), violations)
    }

    /// Legacy support for string-based check (used in some places)
    pub fn is_dangerous_command(full_command: &str) -> (bool, Vec<String>) {
        let parts: Vec<String> = full_command
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();
        if parts.is_empty() {
            return (true, Vec::new());
        }
        let (cmd, args) = parts.split_first().unwrap();
        Self::check(cmd, args)
    }
}
