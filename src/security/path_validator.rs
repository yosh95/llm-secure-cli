use crate::config::CONFIG_MANAGER;
use std::path::PathBuf;

#[derive(Debug)]
pub struct PathValidationError(pub String);

impl std::fmt::Display for PathValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for PathValidationError {}

/// validate_path ensures a path is normalized and check if it falls under
/// the authorized areas. Platform-specific hacks are removed in favor of
/// LLM-based semantic security checks.
pub fn validate_path(path_str: &str) -> Result<PathBuf, PathValidationError> {
    let path_str = path_str.trim().trim_matches('\'').trim_matches('"').trim();

    // 1. Basic Traversal Check (Primitive but effective fast-fail)
    if path_str.contains("..") {
        return Err(PathValidationError(
            "Access to path is forbidden (traversal).".to_string(),
        ));
    }

    let config = CONFIG_MANAGER.get_config();
    let security_config = &config.security;

    // 2. Resolve Path
    let path = if let Some(after_tilde) = path_str.strip_prefix('~') {
        if let Some(home) = dirs::home_dir() {
            home.join(after_tilde.trim_start_matches(['/', '\\']))
        } else {
            PathBuf::from(path_str)
        }
    } else {
        PathBuf::from(path_str)
    };

    // Use absolute path for comparison
    let abs_path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };

    // 3. Simple Whitelist Check
    // We no longer maintain complex blocklists here.
    // The LLM Verifier will see this path in the context and judge if it's sensitive.
    let mut is_allowed = false;
    for allowed in &security_config.allowed_paths {
        let allowed_root = if allowed == "." {
            std::env::current_dir().unwrap_or_default()
        } else {
            PathBuf::from(allowed)
        };

        // Simplified check: falls under an allowed root?
        if abs_path.starts_with(&allowed_root) || abs_path == allowed_root {
            is_allowed = true;
            break;
        }
    }

    if !is_allowed {
        return Err(PathValidationError(format!(
            "Path '{}' is outside allowed directories.",
            path_str
        )));
    }

    Ok(abs_path)
}
