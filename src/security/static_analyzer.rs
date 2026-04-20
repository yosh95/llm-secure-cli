use once_cell::sync::Lazy;
use regex::Regex;

pub struct StaticAnalyzer;

static DANGEROUS_PATTERNS: Lazy<Vec<(Regex, &'static str)>> = Lazy::new(|| {
    vec![
        (
            Regex::new(r"(?i)rm\s+-[rfj]+\s+/").unwrap(),
            "Destructive removal of root directory",
        ),
        (
            Regex::new(r"(?i)mkfs(\s+|\.[a-z0-9]+)").unwrap(),
            "Filesystem formatting",
        ),
        (
            Regex::new(r"(?i)dd\s+if=").unwrap(),
            "Low-level disk writing",
        ),
        (
            Regex::new(r"(?i)>\s*/etc/").unwrap(),
            "Writing to system configuration",
        ),
        (
            Regex::new(r"(?i)chmod\s+(-R\s+)?777").unwrap(),
            "Insecure permission change",
        ),
        (Regex::new(r"(?i)chown\s+").unwrap(), "Ownership change"),
        (
            Regex::new(r"(?i)passwd\s+").unwrap(),
            "Password modification",
        ),
        (
            Regex::new(r"(?i)kill\s+-9").unwrap(),
            "Forceful process termination",
        ),
        (
            Regex::new(r"(?i)curl\s+.*\s+\|\s*sh").unwrap(),
            "Piping remote script to shell",
        ),
    ]
});

impl StaticAnalyzer {
    pub fn is_dangerous_command(command: &str) -> (bool, Vec<String>) {
        let mut violations = Vec::new();

        for (regex, description) in DANGEROUS_PATTERNS.iter() {
            if regex.is_match(command) {
                violations.push(format!("{}: {}", description, regex.as_str()));
            }
        }

        (violations.is_empty(), violations)
    }
}
