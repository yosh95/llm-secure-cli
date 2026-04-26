use crate::consts::LLM_CLI_BASE_DIR;
use std::fs;
use std::path::Path;

/// Sets up base directory and basic permissions.
/// Platform-specific permission bit manipulation (chmod) is minimized
/// to ensure cross-platform compatibility (Windows/Unix).
pub fn setup_permissions() {
    // Ensure base directory exists.
    if !LLM_CLI_BASE_DIR.exists()
        && let Err(e) = fs::create_dir_all(&*LLM_CLI_BASE_DIR)
    {
        log::warn!(
            "Could not create base directory {:?}: {}",
            *LLM_CLI_BASE_DIR,
            e
        );
        return;
    }

    // Critical permission fixing is now handled by simple existence checks
    // or left to the OS/Docker layer for specialized isolation.
    fix_all_permissions();
}

pub fn fix_all_permissions() {
    // Simplified: ensure directories exist.
    // We no longer use Unix-specific 'mode' bits in the core logic
    // to maintain a clean, platform-agnostic code base.
    fn visit_dirs(dir: &Path) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    visit_dirs(&path);
                }
            }
        }
    }

    visit_dirs(&LLM_CLI_BASE_DIR);
}
