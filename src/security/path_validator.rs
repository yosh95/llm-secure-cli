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

fn strip_unc_prefix(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let path_str = path.to_string_lossy();
        if path_str.starts_with(r"\\?\") {
            return PathBuf::from(&path_str[4..]);
        }
    }
    path
}

/// validate_path ensures a path is normalized and checks if it falls under
/// the authorized areas. We use OS-level canonicalization to resolve symlinks
/// and maintain a strict physical boundary.
pub fn validate_path(
    path_str: &str,
    config: &crate::config::models::SecurityConfig,
) -> Result<PathBuf, PathValidationError> {
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
    // We walk up the directory tree until we find a path that exists on disk,
    // then canonicalize it to resolve symlinks, and append the non-existent tail.
    let mut parts = Vec::new();
    let mut current = cleaned_path.clone();
    let canonical_path = loop {
        if let Ok(canonical) = std::fs::canonicalize(&current) {
            let mut res = canonical;
            for part in parts.into_iter().rev() {
                res.push(part);
            }
            break res;
        }
        if let Some(file_name) = current.file_name() {
            parts.push(file_name.to_os_string());
        }
        if let Some(parent) = current.parent() {
            if parent == current {
                break cleaned_path; // Root reached
            }
            current = parent.to_path_buf();
        } else {
            break cleaned_path;
        }
    };

    let security_config = config;

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

        let (cp, ar) = (
            strip_unc_prefix(canonical_path.clone()),
            strip_unc_prefix(allowed_root),
        );

        // Check if the resolved path falls under an allowed root
        if cp.starts_with(&ar) || cp == ar {
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
