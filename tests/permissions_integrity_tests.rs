//! # Permissions Integrity Tests
//!
//! These tests verify that `permissions.rs` correctly enforces the
//! principle of least privilege on the filesystem layer.
//!
//! ## Threat model
//!
//! An attacker with local filesystem access (non-root) should not be able
//! to read sensitive files (.env, private keys) due to overly permissive
//! modes.  Each scenario guards against a specific permission misconfiguration.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use llm_secure_cli::security::permissions::{fix_all_permissions, setup_permissions};
use std::fs;
use std::sync::Mutex;
use std::sync::Once;
use std::sync::OnceLock;
use tempfile::tempdir;

static PERM_LOCK: Mutex<()> = Mutex::new(());
static TEST_ENV_INIT: Once = Once::new();
static _TEST_DIR: OnceLock<tempfile::TempDir> = OnceLock::new();

fn setup_test_env() {
    TEST_ENV_INIT.call_once(|| {
        let dir = tempdir().expect("Failed to create temp dir for test keys");
        let base_path = dir.path().to_path_buf();
        llm_secure_cli::consts::init_base_dir(Some(base_path));
        llm_secure_cli::security::identity::IdentityManager::ensure_keys_with_passphrase(None)
            .expect("Failed to generate PQC keys for test");
        let _ = _TEST_DIR.set(dir);
    });
}

#[cfg(unix)]
fn get_mode(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .map(|m| m.permissions().mode())
        .unwrap_or(0)
}

// =============================================================================
// 1. .env ファイルのパーミッション
// =============================================================================

#[test]
fn test_fix_all_permissions_sets_env_to_600() {
    #[cfg(unix)]
    {
        let _lock = PERM_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        setup_test_env();

        // Create a .env file in base_dir
        let base_dir = llm_secure_cli::consts::get_base_dir();
        let env_path = base_dir.join(".env");
        fs::write(&env_path, "SECRET=value\n").expect("Failed to write .env");

        // Set permissive mode first
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&env_path, fs::Permissions::from_mode(0o644))
                .expect("Failed to set permissive mode");
        }

        fix_all_permissions();

        #[cfg(unix)]
        {
            let mode = get_mode(&env_path);
            assert_eq!(
                mode & 0o777,
                0o600,
                ".env file must be 0o600 after fix_all_permissions, got 0o{:o}",
                mode
            );
        }
    }
}

#[test]
fn test_fix_all_permissions_missing_env_does_not_panic() {
    let _lock = PERM_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    setup_test_env();

    // No .env file exists — fix_all_permissions should not panic
    fix_all_permissions();
    // If we reach here, test passed
}

// =============================================================================
// 2. ディレクトリのパーミッション（再帰的）
// =============================================================================

#[test]
fn test_setup_permissions_creates_base_dir() {
    #[cfg(unix)]
    {
        let _lock = PERM_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        setup_test_env();

        let base_dir = llm_secure_cli::consts::get_base_dir();
        assert!(base_dir.exists(), "Base directory must exist after setup");

        let mode = get_mode(base_dir);
        assert_eq!(
            mode & 0o777,
            0o700,
            "Base directory must be 0o700, got 0o{:o}",
            mode
        );
    }
}

#[test]
fn test_fix_all_permissions_recurse_into_subdirs() {
    #[cfg(unix)]
    {
        let _lock = PERM_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        setup_test_env();

        let base_dir = llm_secure_cli::consts::get_base_dir();

        // Create nested dir structure with permissive modes
        let sub_dir = base_dir.join("subdir");
        fs::create_dir_all(&sub_dir).expect("Failed to create subdir");
        let nested_file = sub_dir.join("key.pem");
        fs::write(&nested_file, "dummy key data").expect("Failed to write nested file");

        // Set permissive modes
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&sub_dir, fs::Permissions::from_mode(0o755))
            .expect("Failed to set permissive dir mode");
        fs::set_permissions(&nested_file, fs::Permissions::from_mode(0o644))
            .expect("Failed to set permissive file mode");

        fix_all_permissions();

        // Subdir should be 0o700
        let dir_mode = get_mode(&sub_dir);
        assert_eq!(
            dir_mode & 0o777,
            0o700,
            "Subdirectory must be 0o700 after fix, got 0o{:o}",
            dir_mode
        );

        // Nested file should be 0o600
        let file_mode = get_mode(&nested_file);
        assert_eq!(
            file_mode & 0o777,
            0o600,
            "Nested file must be 0o600 after fix, got 0o{:o}",
            file_mode
        );
    }
}

// =============================================================================
// 3. ルート .env の保護
// =============================================================================

#[test]
fn test_fix_all_permissions_handles_root_dotenv() {
    #[cfg(unix)]
    {
        let _lock = PERM_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        setup_test_env();

        // Create a .env in the current working directory
        // (This is the "root" .env that fix_all_permissions also checks)
        let root_env = std::path::Path::new(".env");
        if !root_env.exists() {
            fs::write(root_env, "ROOT_SECRET=value\n").expect("Failed to write root .env");
        }

        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root_env, fs::Permissions::from_mode(0o644))
            .expect("Failed to set permissive root .env mode");

        fix_all_permissions();

        let mode = get_mode(root_env);
        assert_eq!(
            mode & 0o777,
            0o600,
            "Root .env must be 0o600 after fix, got 0o{:o}",
            mode
        );

        // Cleanup
        let _ = fs::remove_file(root_env);
    }
}

// =============================================================================
// 4. 存在しないベースディレクトリからの復旧
// =============================================================================

#[test]
fn test_setup_permissions_does_not_panic_on_call() {
    #[cfg(unix)]
    {
        let _lock = PERM_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        setup_test_env();

        // setup_permissions is idempotent — calling it again must not panic
        setup_permissions();

        let base_dir = llm_secure_cli::consts::get_base_dir();
        assert!(base_dir.exists(), "Base directory must exist");

        let mode = get_mode(base_dir);
        assert_eq!(
            mode & 0o777,
            0o700,
            "Base directory must be 0o700, got 0o{:o}",
            mode
        );
    }
}
