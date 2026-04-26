use crate::consts::LLM_CLI_BASE_DIR;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

pub fn setup_permissions() {
    // 1. Set umask so newly created files/dirs are restricted by default.
    #[cfg(unix)]
    unsafe {
        libc::umask(0o077);
    }

    // 2. Ensure base directory exists with correct permissions.
    if !LLM_CLI_BASE_DIR.exists() {
        if let Err(e) = fs::create_dir_all(&*LLM_CLI_BASE_DIR) {
            log::warn!(
                "Could not create base directory {:?}: {}",
                *LLM_CLI_BASE_DIR,
                e
            );
            return;
        }
        #[cfg(unix)]
        ensure_mode(&LLM_CLI_BASE_DIR, 0o700);
    } else {
        #[cfg(unix)]
        ensure_mode(&LLM_CLI_BASE_DIR, 0o700);
    }

    // 3. Fix permissions for critical subdirectories and files.
    fix_all_permissions();
}

#[cfg(unix)]
fn ensure_mode(path: &Path, mode: u32) {
    if let Ok(metadata) = path.metadata() {
        let current_mode = metadata.permissions().mode() & 0o777;
        if current_mode != mode {
            let mut perms = metadata.permissions();
            perms.set_mode(mode);
            if let Err(e) = fs::set_permissions(path, perms) {
                log::debug!("Failed to set permissions for {:?}: {}", path, e);
            }
        }
    }
}

pub fn fix_all_permissions() {
    // Using simple recursion for now to avoid adding walkdir if not strictly necessary
    fn visit_dirs(dir: &Path) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    #[cfg(unix)]
                    ensure_mode(&path, 0o700);
                    visit_dirs(&path);
                } else {
                    #[cfg(unix)]
                    ensure_mode(&path, 0o600);
                }
            }
        }
    }

    visit_dirs(&LLM_CLI_BASE_DIR);
}
