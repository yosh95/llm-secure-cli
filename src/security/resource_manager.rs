use std::process::Child;

/// Set resource limits for the current process.
/// In the modern Docker-centric model, we offload hard resource enforcement
/// to the container runtime (e.g., --memory, --cpus).
/// This function is now a stub to maintain API compatibility and simplify OS-specific code.
pub fn set_resource_limits(_mem_limit_mb: u64, _cpu_limit_sec: u64, _file_limit_mb: u64) {
    // No-op: Offloaded to Docker or OS-native limits (if applicable).
    // This avoids using 'libc' and platform-specific rlimit calls.
}

/// Best-effort resource limiting for a child process.
pub fn limit_process_resources(_child: &Child, _mem_limit_mb: u64) {
    // No-op: Offloaded to containerization or higher-level OS management.
}
