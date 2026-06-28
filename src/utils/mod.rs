pub mod chat_logger;
pub mod http;
pub mod logging;
pub mod media;
pub mod session_store;

const HEX_CHARS: [char; 16] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f',
];

pub fn hex_encode(data: impl AsRef<[u8]>) -> String {
    let bytes = data.as_ref();
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX_CHARS[((b >> 4) & 0x0f) as usize]);
        s.push(HEX_CHARS[(b & 0x0f) as usize]);
    }
    s
}

pub fn format_number<T: std::fmt::Display>(n: T) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut result = String::new();

    for (i, &b) in bytes.iter().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(b as char);
    }
    result.chars().rev().collect()
}

/// Restore the terminal to cooked mode so that Ctrl+C (SIGINT) works correctly.
///
/// rustyline leaves the terminal in raw mode on some code paths (e.g. on
/// interrupt or error), which disables ISIG and makes Ctrl+C unable to
/// generate SIGINT.  This function resets the terminal to sane settings
/// using `stty sane` and explicitly re-enables `icanon` and `isig`.
pub fn restore_terminal() {
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("stty").args(["sane"]).status();
        let _ = std::process::Command::new("stty")
            .args(["icanon", "isig"])
            .status();
    }
}

/// Ensure the ISIG flag is enabled so that Ctrl+C generates SIGINT.
///
/// Call this before any blocking operation (tool execution, HTTP request)
/// that should be responsive to Ctrl+C.  This is a safety net in case
/// rustyline or another component left the terminal in raw mode.
pub fn ensure_isig_enabled() {
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("stty").args(["isig"]).status();
    }
}
