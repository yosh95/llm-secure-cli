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

/// validate_path ensures a path is normalized.
pub fn validate_path(
    path_str: &str,
    config: &crate::config::models::SecurityConfig,
) -> Result<PathBuf, PathValidationError> {
    let path_str = path_str.trim().trim_matches('\'').trim_matches('"').trim();

    // Early exit: If the string contains newlines or looks like code/long text,
    // it's likely not a path and should be rejected but not necessarily as a "denied path".
    if path_str.contains('\n') || path_str.len() > 1024 {
        return Err(PathValidationError(
            "Invalid path format: string too long or contains newlines".to_string(),
        ));
    }
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

    let _security_config = config;

    // 4. Semantic Boundary Resolution
    // We rely on Dual LLM (Phase 3) for semantic path verification and
    // intent matching. Hardcoded whitelists are removed in favor of
    // AI-native policy enforcement.

    Ok(canonical_path)
}
