use crate::config::ConfigManager;
use crate::consts::chat_log_path;
use crate::llm::models::Role;
use jiff::Timestamp;
use std::fs::{self, OpenOptions};
use std::io::Write;

pub fn log_chat(
    config_manager: &ConfigManager,
    role: &Role,
    content: &str,
    model_name: Option<&str>,
) {
    let path_val = chat_log_path();
    let path = &path_val;
    let config = match config_manager.get_config() {
        Ok(c) => c,
        Err(_) => return,
    };
    let max_lines = config.general.max_chat_log_lines;

    // Ensure directory exists
    if let Some(parent) = path.parent()
        && !parent.exists()
        && let Err(e) = fs::create_dir_all(parent)
    {
        tracing::error!("Failed to create chat log directory: {}", e);
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Some(p) = path.parent()
            && !p.exists()
            && let Err(e) = fs::set_permissions(p, fs::Permissions::from_mode(0o700))
        {
            tracing::warn!("Failed to set permissions on {:?}: {}", p, e);
        }
    }

    let timestamp = Timestamp::now().to_string();
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

    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    if let Ok(mut file) = options.open(path)
        && let Err(e) = file.write_all(log_entry.as_bytes())
    {
        tracing::error!("Failed to write chat log entry: {}", e);
    }

    // Rotation
    if let Ok(metadata) = fs::metadata(path) {
        // Rough estimate of line count
        let estimated_lines = metadata.len() / 200;
        if estimated_lines > (max_lines as u64 * 11 / 10) {
            trim_chat_log(config_manager, path, max_lines);
        }
    }
}

fn trim_chat_log(config_manager: &ConfigManager, path: &std::path::Path, max_lines: usize) {
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

    let config = match config_manager.get_config() {
        Ok(c) => c,
        Err(_) => return,
    };
    let max_archives = config.general.max_chat_archives;

    if max_archives > 0
        && let Err(e) = crate::utils::logging::rotate_file(path, max_archives)
    {
        tracing::warn!("Failed to rotate chat log: {}", e);
    }

    let kept_lines = &lines[lines.len() - max_lines..];
    if let Ok(mut file) = std::fs::File::create(path) {
        for line in kept_lines {
            if let Err(e) = writeln!(file, "{line}") {
                tracing::warn!("Failed to write chat log line: {}", e);
            }
        }
    }
}
