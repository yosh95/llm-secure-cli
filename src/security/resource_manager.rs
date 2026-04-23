#[cfg(unix)]
use libc::{RLIMIT_AS, RLIMIT_CPU, RLIMIT_FSIZE, rlimit, setrlimit};
use std::process::Child;

pub fn set_resource_limits(mem_limit_mb: u64, cpu_limit_sec: u64, file_limit_mb: u64) {
    #[cfg(unix)]
    unsafe {
        // CPU time limit
        let cpu_limit = rlimit {
            rlim_cur: cpu_limit_sec,
            rlim_max: cpu_limit_sec + 5,
        };
        setrlimit(RLIMIT_CPU, &cpu_limit);

        // Address space (Memory) limit
        // Skip on Android/Termux if necessary (detection logic needed or just try and ignore error)
        let mem_limit = mem_limit_mb * 1024 * 1024;
        let mem_rlimit = rlimit {
            rlim_cur: mem_limit,
            rlim_max: mem_limit,
        };
        setrlimit(RLIMIT_AS, &mem_rlimit);

        // File size limit
        let file_limit = file_limit_mb * 1024 * 1024;
        let file_rlimit = rlimit {
            rlim_cur: file_limit,
            rlim_max: file_limit,
        };
        setrlimit(RLIMIT_FSIZE, &file_rlimit);
    }
}

pub fn limit_process_resources(_child: &Child, _mem_limit_mb: u64) {
    // best effort: set priority/niceness
    #[cfg(unix)]
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, _child.id() as libc::id_t, 10);
    }

    // On Windows, one might use Job Objects or other APIs,
    // but without extra crates it's complex.
}
