use crate::config::CONFIG_MANAGER;
use crate::consts::LLM_CLI_BASE_DIR;
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

pub fn validate_path(path_str: &str) -> Result<PathBuf, PathValidationError> {
    let path_str = path_str.trim().trim_matches('\'').trim_matches('"').trim();

    if path_str.contains("..") {
        return Err(PathValidationError(
            "Access to path is forbidden (traversal).".to_string(),
        ));
    }

    let config = CONFIG_MANAGER.get_config();
    let security_config = &config.security;

    // Expand user if path starts with ~
    let path = if let Some(after_tilde) = path_str.strip_prefix('~') {
        if let Some(home) = dirs::home_dir() {
            // strip the leading '~' then any single '/' or '\' separator
            let rest = after_tilde.trim_start_matches(['/', '\\']);
            if rest.is_empty() {
                home
            } else {
                home.join(rest)
            }
        } else {
            PathBuf::from(path_str)
        }
    } else {
        PathBuf::from(path_str)
    };

    // Canonicalize or Clean
    let abs_path = if path.exists() {
        path.canonicalize().map_err(|e| {
            PathValidationError(format!("Failed to resolve path (canonicalize): {}", e))
        })?
    } else {
        let path_obj = path.clean();
        if path_obj.is_absolute() {
            path_obj
        } else {
            std::env::current_dir()
                .unwrap_or_default()
                .join(path_obj)
                .clean()
        }
    };

    // 2. Block sensitive paths
    if abs_path == *LLM_CLI_BASE_DIR || abs_path.starts_with(&*LLM_CLI_BASE_DIR) {
        return Err(PathValidationError(
            "Access to path is forbidden (base dir).".to_string(),
        ));
    }

    for blocked in &security_config.blocked_paths {
        let b_path = PathBuf::from(blocked).clean();
        let b_abs = if b_path.is_absolute() {
            b_path
        } else {
            std::env::current_dir()
                .unwrap_or_default()
                .join(b_path)
                .clean()
        };
        if abs_path == b_abs || abs_path.starts_with(&b_abs) {
            return Err(PathValidationError(format!(
                "Access to blocked path is forbidden: {}",
                blocked
            )));
        }
    }

    // 3. Filename blocklist
    if let Some(filename) = abs_path.file_name().and_then(|s| s.to_str()) {
        for pattern in &security_config.blocked_filenames {
            let glob = regex::Regex::new(&pattern.replace("*", ".*").replace("?", ".")).unwrap();
            if glob.is_match(filename) {
                return Err(PathValidationError(format!(
                    "Access to filename '{}' is forbidden.",
                    filename
                )));
            }
        }
    }

    // 4. Whitelist check
    let mut is_allowed = false;
    for allowed in &security_config.allowed_paths {
        if allowed == "." {
            let cwd = std::env::current_dir().unwrap_or_default();
            if abs_path == cwd || abs_path.starts_with(&cwd) {
                is_allowed = true;
                break;
            }
            continue;
        }

        let a_path = PathBuf::from(allowed).clean();
        let a_abs = if a_path.is_absolute() {
            a_path
        } else {
            std::env::current_dir()
                .unwrap_or_default()
                .join(a_path)
                .clean()
        };
        if abs_path == a_abs || abs_path.starts_with(&a_abs) {
            is_allowed = true;
            break;
        }
    }

    if !is_allowed {
        return Err(PathValidationError(
            "Access to path is not in the whitelist.".to_string(),
        ));
    }

    Ok(abs_path)
}
