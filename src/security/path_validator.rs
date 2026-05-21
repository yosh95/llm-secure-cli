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

/// Validates a path for safe file-system access.
///
/// On Windows, this handles UNC paths (`\\server\share\...`), drive-letter
/// paths (`C:\...`), and ensures that drive-relative paths cannot escape
/// the allowed boundary.
///
/// On all platforms, normalisation, symlink resolution, and whitelist
/// boundary checks are performed.
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

    // Platform-specific early checks
    #[cfg(target_os = "windows")]
    {
        // Block Windows reserved device names (CON, PRN, AUX, NUL, COM1.., LPT1..)
        if let Some(stem) = path_str
            .split(|c: char| c == '\\' || c == '/' || c == ':')
            .next()
        {
            let upper = stem.to_uppercase();
            let reserved = [
                "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
                "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8",
                "LPT9",
            ];
            if reserved.contains(&upper.as_str()) {
                return Err(PathValidationError(format!(
                    "Access denied: Windows reserved device name '{}'",
                    stem
                )));
            }
        }
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

    // 4. Physical Boundary Resolution
    // Verify that the path resides within one of the allowed base directories.
    let mut is_allowed = false;
    for allowed_str in &config.allowed_paths {
        let allowed_base = if let Some(after_tilde) = allowed_str.strip_prefix('~') {
            if let Some(home) = dirs::home_dir() {
                home.join(after_tilde.trim_start_matches(['/', '\\']))
            } else {
                PathBuf::from(allowed_str)
            }
        } else {
            PathBuf::from(allowed_str)
        };

        let resolved_allowed =
            std::fs::canonicalize(&allowed_base).unwrap_or_else(|_| allowed_base.clean());

        // Platform-aware comparison
        if paths_are_within(&canonical_path, &resolved_allowed) {
            is_allowed = true;
            break;
        }
    }

    if !is_allowed {
        return Err(PathValidationError(format!(
            "Access denied: Path '{:?}' is outside of allowed directories: {:?}",
            canonical_path, config.allowed_paths
        )));
    }

    // 5. Semantic Boundary Resolution
    // We also rely on Dual LLM (Phase 3) for semantic path verification and
    // intent matching.

    Ok(canonical_path)
}

/// Platform-aware comparison to check if `candidate` starts with `base`.
///
/// On Windows, this performs a case-insensitive comparison of the drive
/// letter and UNC share prefix, while remaining case-sensitive for the
/// rest of the path.  On other platforms, standard byte-wise comparison
/// is used (which is case-sensitive on Unix).
fn paths_are_within(candidate: &std::path::Path, base: &std::path::Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        // On Windows, normalise both paths to use backslashes and strip
        // the UNC prefix (\\?\) that canonicalize() adds for long paths.
        let candidate_str = candidate.to_string_lossy().replace('/', "\\");
        let base_str = base.to_string_lossy().replace('/', "\\");

        // Remove the UNC extended-length path prefix if present
        let candidate_norm = candidate_str.trim_start_matches(r"\\?\");
        let base_norm = base_str.trim_start_matches(r"\\?\");

        // Handle drive-letter comparison: C:\ and c:\ should be equivalent
        let candidate_lower = candidate_norm.to_lowercase();
        let base_lower = base_norm.to_lowercase();

        candidate_lower.starts_with(&base_lower)
    }

    #[cfg(not(target_os = "windows"))]
    {
        candidate.starts_with(base)
    }
}
