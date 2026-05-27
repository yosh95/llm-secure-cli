use crate::consts::get_base_dir;
use std::fs;
use std::path::Path;

/// Sets up base directory and basic permissions.
/// Platform-specific permission bit manipulation (chmod) is minimized
/// to ensure cross-platform compatibility (Windows/Unix).
pub fn setup_permissions() {
    let base_dir = get_base_dir();
    // Ensure base directory exists.
    if !base_dir.exists()
        && let Err(_e) = fs::create_dir_all(base_dir)
    {
        return;
    }

    // Critical permission fixing is now handled by simple existence checks
    // or left to the OS/Docker layer for specialized isolation.
    fix_all_permissions();
}

pub fn fix_all_permissions() {
    let base_dir = get_base_dir();

    // 1. Set 600 for .env files in root and base_dir
    let dotenv_paths = [Path::new(".env").to_owned(), base_dir.join(".env")];
    for path in dotenv_paths {
        if path.exists()
            && let Err(e) = set_private_permissions(&path)
        {
            tracing::warn!("Failed to set private permissions on {:?}: {}", path, e);
        }
    }

    // 2. Recursively set permissions for base_dir
    if base_dir.exists() {
        if let Err(e) = set_dir_private_permissions(base_dir) {
            tracing::warn!(
                "Failed to set directory permissions on {:?}: {}",
                base_dir,
                e
            );
        }
        recursive_set_permissions(base_dir);
    }
}

fn recursive_set_permissions(dir: &Path) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Err(e) = set_dir_private_permissions(&path) {
                    tracing::warn!("Failed to set directory permissions on {:?}: {}", path, e);
                }
                recursive_set_permissions(&path);
            } else {
                if let Err(e) = set_private_permissions(&path) {
                    tracing::warn!("Failed to set private permissions on {:?}: {}", path, e);
                }
            }
        }
    }
}

fn set_private_permissions(_path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(_path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn set_dir_private_permissions(_path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(_path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
