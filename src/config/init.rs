use std::fs;

use crate::consts::{CONFIG_DIR, CONFIG_FILE_PATH};

const DEFAULTS: &str = include_str!("defaults.toml");

pub fn init_config() {
    if CONFIG_FILE_PATH.exists() {
        return;
    }

    if let Err(e) = fs::create_dir_all(&*CONFIG_DIR) {
        eprintln!(
            "Error: Could not create config directory {:?}: {}",
            *CONFIG_DIR, e
        );
        return;
    }

    let mut commented_lines = String::new();
    commented_lines.push_str("# llm-secure-cli Configuration\n");
    let sep = format!("# {}\n", "=".repeat(77));
    commented_lines.push_str(&sep);
    commented_lines.push_str("# IMPORTANT: API Keys are NOT stored in this file.\n");
    commented_lines.push_str("# Set API Keys as env vars (e.g., in ~/.bashrc or .env):\n");
    commented_lines.push_str("#   export OPENAI_API_KEY='your-key-here'\n");
    commented_lines.push_str("#   export GEMINI_API_KEY='your-key-here'\n");
    commented_lines.push_str("#   export ANTHROPIC_API_KEY='your-key-here'\n");
    commented_lines.push_str("#\n");
    commented_lines.push_str(
        "# For Dual LLM Verification, ensure you have keys for TWO different providers.\n",
    );
    commented_lines.push_str(&sep);
    commented_lines.push_str("\n# Other settings can be customized below.\n\n");

    for line in DEFAULTS.lines() {
        let stripped = line.trim();
        if !stripped.is_empty() && !stripped.starts_with('[') && !stripped.starts_with('#') {
            commented_lines.push_str(&format!("# {}\n", line));
        } else {
            commented_lines.push_str(line);
            commented_lines.push('\n');
        }
    }

    if let Err(e) = fs::write(&*CONFIG_FILE_PATH, commented_lines) {
        eprintln!(
            "Error: Could not write config file {:?}: {}",
            *CONFIG_FILE_PATH, e
        );
    } else {
        eprintln!("Initialized config at {:?}", *CONFIG_FILE_PATH);
    }
}
