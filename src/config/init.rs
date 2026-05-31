use crate::cli::ui;
use std::fs;

use crate::consts::{config_dir, config_file_path};

const DEFAULTS: &str = include_str!("defaults.toml");

pub fn init_config() {
    let c_file = config_file_path();
    let c_dir = config_dir();

    if c_file.exists() {
        return;
    }

    if let Err(e) = fs::create_dir_all(&c_dir) {
        ui::report_error(&format!("Could not create config directory {c_dir:?}: {e}"));
        return;
    }

    let mut commented_lines = String::new();
    commented_lines.push_str("# llm-secure-cli Configuration\n");
    let sep = format!("# {}\n", "=".repeat(77));
    commented_lines.push_str(&sep);
    commented_lines.push_str("# IMPORTANT: API Keys are NOT stored in this file.\n");
    commented_lines.push_str("# Set API Keys as env vars (e.g., in ~/.bashrc or .env):\n");
    commented_lines.push_str("#   export OPENAI_API_KEY='your-key-here'\n");
    commented_lines.push_str("#   export OPENROUTER_API_KEY='your-key-here'\n");
    commented_lines.push_str("#\n");
    commented_lines.push_str(
        "# For Verifier Committee, ensure you have keys for the providers you configure.\n",
    );
    commented_lines.push_str(&sep);
    commented_lines.push_str("\n# Other settings can be customized below.\n\n");

    for line in DEFAULTS.lines() {
        let stripped = line.trim();
        if !stripped.is_empty()
            && !stripped.starts_with('#')
            && (!stripped.starts_with('[') || stripped.starts_with("[["))
        {
            commented_lines.push_str(&format!("# {line}\n"));
        } else {
            commented_lines.push_str(line);
            commented_lines.push('\n');
        }
    }

    if let Err(e) = fs::write(&c_file, commented_lines) {
        ui::report_error(&format!("Could not write config file {c_file:?}: {e}"));
    } else {
        ui::report_success(&format!("Initialized config at {c_file:?}"));
    }
}
