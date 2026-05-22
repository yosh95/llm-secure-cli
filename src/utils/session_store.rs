use crate::core::session::ActiveSession;
use crate::llm::models::Message;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Full session file format (superset of the old /save format).
/// The `conversation` field is exactly what `/save` produced.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SessionFile {
    pub trace_id: String,
    pub created_at: String,
    pub model: String,
    pub provider: String,
    pub conversation: Vec<Message>,
}

/// Lightweight info for listing sessions (without loading full conversation).
#[derive(Debug)]
pub struct SessionListing {
    pub filename: String,
    pub trace_id: String,
    pub created_at: String,
    pub model: String,
    pub provider: String,
    pub first_user_prompt: Option<String>,
}

/// Auto-save the current session conversation to the sessions directory.
/// Called after every complete turn (user message + assistant response).
pub fn auto_save(session: &ActiveSession) {
    let dir = crate::consts::sessions_dir();
    if let Err(e) = fs::create_dir_all(&dir) {
        tracing::warn!("Failed to create sessions directory: {}", e);
        return;
    }

    let state = session.get_client().get_state();
    let trace_id = &session.trace_id;

    let session_file = SessionFile {
        trace_id: trace_id.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
        model: state.model.clone(),
        provider: state.provider.clone(),
        conversation: state.conversation.clone(),
    };

    let path = dir.join(format!("{}.json", trace_id));

    let file = match std::fs::File::create(&path) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(
                trace_id = %trace_id,
                error = %e,
                "Failed to create session file"
            );
            return;
        }
    };
    match serde_json::to_writer_pretty(&file, &session_file) {
        Ok(_) => {}
        Err(e) => {
            tracing::warn!(
                trace_id = %trace_id,
                error = %e,
                "Failed to auto-save session"
            );
        }
    }

    // Set restrictive permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
}

/// List all available sessions with metadata for display.
pub fn list_sessions() -> anyhow::Result<Vec<SessionListing>> {
    let dir = crate::consts::sessions_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut listings = Vec::new();

    for entry in fs::read_dir(&dir)? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let path = entry.path();

        // Only consider .json files
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }

        let filename = match path.file_stem().and_then(|s| s.to_str()) {
            Some(f) => f.to_string(),
            None => continue,
        };

        // Read the file
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        // Try new format first, fall back to old format (bare conversation array)
        let (trace_id, created_at, model, provider, first_user_prompt) =
            if let Ok(sf) = serde_json::from_str::<SessionFile>(&content) {
                let first = extract_first_user_prompt(&sf.conversation);
                (sf.trace_id, sf.created_at, sf.model, sf.provider, first)
            } else if let Ok(conv) = serde_json::from_str::<Vec<Message>>(&content) {
                let first = extract_first_user_prompt(&conv);
                (
                    filename.clone(),
                    String::new(),
                    String::new(),
                    String::new(),
                    first,
                )
            } else {
                continue;
            };

        listings.push(SessionListing {
            filename,
            trace_id,
            created_at,
            model,
            provider,
            first_user_prompt,
        });
    }

    // Sort by created_at ascending (oldest first)
    listings.sort_by(|a, b| a.created_at.cmp(&b.created_at));

    Ok(listings)
}

/// Load a session from the sessions directory by its filename (without .json extension).
pub fn load_session(filename: &str) -> anyhow::Result<Vec<Message>> {
    let dir = crate::consts::sessions_dir();

    // Try with exact filename, then with .json appended
    let path: PathBuf = if filename.ends_with(".json") {
        let p = dir.join(filename);
        if p.exists() {
            p
        } else {
            dir.join(filename) // fallback
        }
    } else {
        let p = dir.join(format!("{}.json", filename));
        if p.exists() {
            p
        } else {
            // Try as absolute or relative path (for backward compat)
            let p = PathBuf::from(filename);
            if p.exists() {
                return load_from_path(&p);
            }
            return Err(anyhow::anyhow!(
                "Session '{}' not found in {}",
                filename,
                dir.display()
            ));
        }
    };

    load_from_path(&path)
}

/// Load a session from an arbitrary path (for --session CLI arg backward compatibility).
pub fn load_from_path(path: &std::path::Path) -> anyhow::Result<Vec<Message>> {
    let content = fs::read_to_string(path)?;

    // Try new SessionFile format first, then old bare array format
    if let Ok(sf) = serde_json::from_str::<SessionFile>(&content) {
        Ok(sf.conversation)
    } else {
        let conversation: Vec<Message> = serde_json::from_str(&content)?;
        Ok(conversation)
    }
}

/// Get the path to a session file by its trace_id.
pub fn session_path(trace_id: &str) -> PathBuf {
    crate::consts::sessions_dir().join(format!("{}.json", trace_id))
}

/// Delete a single session by its filename (without .json extension).
/// Returns Ok(true) if deleted, Ok(false) if not found.
pub fn delete_session(filename: &str) -> anyhow::Result<bool> {
    let dir = crate::consts::sessions_dir();
    if !dir.exists() {
        return Ok(false);
    }

    let path = if filename.ends_with(".json") {
        dir.join(filename)
    } else {
        dir.join(format!("{}.json", filename))
    };

    if path.exists() {
        fs::remove_file(&path)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Delete all session files from the sessions directory.
/// Returns the number of files deleted.
pub fn clear_sessions() -> anyhow::Result<usize> {
    let dir = crate::consts::sessions_dir();
    if !dir.exists() {
        return Ok(0);
    }

    let mut count = 0;
    for entry in fs::read_dir(&dir)? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            fs::remove_file(&path)?;
            count += 1;
        }
    }
    Ok(count)
}

fn extract_first_user_prompt(conversation: &[Message]) -> Option<String> {
    use crate::llm::models::Role;
    conversation.iter().find(|m| m.role == Role::User).map(|m| {
        let text = m.get_text(false);
        // Truncate to first line or ~50 chars
        if let Some(first_line) = text.lines().next() {
            if first_line.chars().count() > 50 {
                format!("{}...", first_line.chars().take(47).collect::<String>())
            } else {
                first_line.to_string()
            }
        } else {
            String::new()
        }
    })
}
