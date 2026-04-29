use crate::config::CONFIG_MANAGER;
use crate::consts::CHAT_LOG_PATH;
use crate::llm::models::Role;
use chrono::Utc;
use std::fs::{self, OpenOptions};
use std::io::Write;

pub fn log_chat(role: &Role, content: &str, model_name: Option<&str>) {
    let path = &*CHAT_LOG_PATH;
    let config = CONFIG_MANAGER.get_config();
    let max_lines = config.general.max_chat_log_lines;

    // Ensure directory exists
    if let Some(parent) = path.parent()
        && !parent.exists()
    {
        let _ = fs::create_dir_all(parent);
    }

    let timestamp = Utc::now().to_rfc3339();
    let role_str = match role {
        Role::User => "USER",
        Role::Assistant | Role::Model => model_name.unwrap_or("ASSISTANT"),
        Role::System => "SYSTEM",
        Role::Tool => "TOOL",
    };

    let log_entry = format!(
        "[{}] [{}] {}\n",
        timestamp,
        role_str,
        content.replace('\n', " ")
    );

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(log_entry.as_bytes());
    }

    // Rotation
    if let Ok(metadata) = fs::metadata(path) {
        // Rough estimate of line count
        let estimated_lines = metadata.len() / 200;
        if estimated_lines > (max_lines as u64 * 11 / 10) {
            trim_chat_log(path, max_lines);
        }
    }
}

fn trim_chat_log(path: &std::path::Path, max_lines: usize) {
    if !path.exists() {
        return;
    }

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };

    let lines: Vec<&str> = content.lines().collect();
    if lines.len() <= max_lines {
        return;
    }

    let config = CONFIG_MANAGER.get_config();
    let max_archives = config.general.max_chat_archives;

    if max_archives > 0 {
        let _ = crate::utils::logging::rotate_file(path, max_archives);
    }

    let kept_lines = &lines[lines.len() - max_lines..];
    if let Ok(mut file) = std::fs::File::create(path) {
        for line in kept_lines {
            let _ = writeln!(file, "{}", line);
        }
    }
}
