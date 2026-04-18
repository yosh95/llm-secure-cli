use regex::Regex;

pub struct StaticAnalyzer;

impl StaticAnalyzer {
    pub fn analyze_python_safety(code: &str) -> (bool, Vec<String>, Vec<String>) {
        let mut violations = Vec::new();
        let mut warnings = Vec::new();

        // 1. Block dangerous modules
        let dangerous_modules = [
            "os",
            "subprocess",
            "pty",
            "socket",
            "base64",
            "codecs",
            "importlib",
            "shutil",
        ];

        for module in dangerous_modules {
            let re = Regex::new(&format!(r"(import\s+{0}|from\s+{0}\s+import)", module)).unwrap();
            if re.is_match(code) {
                violations.push(format!("Forbidden module import: {}", module));
            }
        }

        // 2. Block dangerous built-ins
        let dangerous_funcs = ["eval", "exec", "compile", "open", "__import__"];
        for func in dangerous_funcs {
            let re = Regex::new(&format!(r"\b{}\(", func)).unwrap();
            if re.is_match(code) {
                violations.push(format!("Forbidden function call: {}", func));
            }
        }

        // 3. Detect obfuscation (e.g., "ex" + "ec")
        if code.contains("+") && (code.contains("\"ex\"") || code.contains("'ex'")) {
            violations.push("Potential obfuscated keyword construction detected".to_string());
        }

        // 4. Reflection attacks
        if code.contains("__subclasses__")
            || code.contains("__base__")
            || code.contains("__globals__")
        {
            violations.push("Potential reflection-based attack detected".to_string());
        }

        // 5. Warnings (Subprocess without shell=True is still risky but maybe allowed)
        if code.contains("subprocess.run") && !code.contains("shell=False") {
            warnings.push("subprocess.run detected without explicit shell=False".to_string());
        }

        (violations.is_empty(), violations, warnings)
    }
}
