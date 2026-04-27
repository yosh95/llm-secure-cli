use crate::config::CONFIG_MANAGER;
use path_clean::PathClean;
use std::path::PathBuf;

#[derive(Debug)]
pub struct PathValidationError(pub String);

impl std::fmt::Display for PathValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for PathValidationError {}

/// validate_path ensures a path is normalized and checks if it falls under
/// the authorized areas. We use OS-level canonicalization to resolve symlinks
/// and maintain a strict physical boundary.
pub fn validate_path(path_str: &str) -> Result<PathBuf, PathValidationError> {
    let path_str = path_str.trim().trim_matches('\'').trim_matches('"').trim();

    // 1. Basic construction
    let mut path = if let Some(after_tilde) = path_str.strip_prefix('~') {
        if let Some(home) = dirs::home_dir() {
            home.join(after_tilde.trim_start_matches(['/', '\\']))
        } else {
            PathBuf::from(path_str)
        }
    } else {
        PathBuf::from(path_str)
    };

    // 2. Resolve to absolute and clean logical traversals
    if !path.is_absolute() {
        path = std::env::current_dir().unwrap_or_default().join(path);
    }
    let cleaned_path = path.clean();

    // 3. Resolve physical entity (Symbolic links)
    // If the file does not exist (e.g. creating a new file), we resolve its
    // parent directory to ensure the root is valid.
    let canonical_path = std::fs::canonicalize(&cleaned_path).unwrap_or(cleaned_path);

    let config = CONFIG_MANAGER.get_config();
    let security_config = &config.security;

    // 4. Simple Whitelist Check
    let mut is_allowed = false;
    for allowed in &security_config.allowed_paths {
        let allowed_root = std::fs::canonicalize(if allowed == "." {
            std::env::current_dir().unwrap_or_default()
        } else {
            PathBuf::from(allowed)
        })
        .unwrap_or_else(|_| {
            if allowed == "." {
                std::env::current_dir().unwrap_or_default()
            } else {
                PathBuf::from(allowed)
            }
        });

        // Check if the resolved path falls under an allowed root
        if canonical_path.starts_with(&allowed_root) || canonical_path == allowed_root {
            is_allowed = true;
            break;
        }
    }

    if !is_allowed {
        let msg = if path_str.contains("..") {
            format!(
                "Access to path '{}' is denied (potential path traversal).",
                path_str
            )
        } else {
            format!(
                "Access to path '{}' is denied (outside allowed directories).",
                path_str
            )
        };
        return Err(PathValidationError(msg));
    }

    Ok(canonical_path)
}
