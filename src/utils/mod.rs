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

/// Remove or replace control characters that can corrupt terminal display.
///
/// Strips ANSI escape sequences (colour, cursor movement, erase operations)
/// and other C0 control characters (except `\t`, `\n`, `\r`) that are
/// harmless or meaningful for display.  `\r` (carriage return) is also
/// removed because it causes the terminal to over-write the current line
/// when a progress bar or similar output is captured.
///
/// This is intended for tool output that will be displayed to the user or
/// sent back to the LLM.  The LLM has no use for control characters either.
pub fn sanitize_for_display(s: &str) -> String {
    // 1. Remove ANSI escape sequences: ESC [ ... <final byte>
    //    CSI sequences: \x1b[ <parameter bytes> <intermediate bytes> <final byte>
    //    where parameter bytes are 0x30-0x3F, intermediate bytes 0x20-0x2F,
    //    final byte 0x40-0x7E.
    //    We also catch ESC but not followed by [ (e.g. ESC 7, ESC 8, etc.)
    #[expect(clippy::expect_used)]
    let re_ansi = regex::bytes::Regex::new(r"(?:\[[0-?]*[ -/]*[@-~]|[ -/]?[@-~])")
        .expect("valid ANSI escape regex");

    let s = re_ansi.replace_all(s.as_bytes(), b"");
    // Safety: we only remove bytes, the result is still valid UTF-8 because
    // we never split multi-byte sequences.
    #[expect(clippy::expect_used)]
    let s = String::from_utf8(s.into_owned()).expect("still valid UTF-8 after ANSI removal");

    // 2. Remove C0 control characters except \t (0x09), \n (0x0a), \r (0x0d).
    //    \r is removed because captured progress-bar output contains many \r
    //    that cause line-overwrite visual corruption when displayed as-is.
    #[expect(clippy::expect_used)]
    let re_c0 =
        regex::Regex::new("[\x00-\x08\x0b\x0c\x0d\x0e-\x1f\x7f]").expect("valid C0 control regex");
    let s = re_c0.replace_all(&s, "");

    s.into_owned()
}

#[cfg(test)]
mod tests {
    use super::sanitize_for_display;

    #[test]
    fn removes_ansi_colour() {
        let result = sanitize_for_display("\x1b[31mred\x1b[0m");
        assert_eq!(result, "red");
    }

    #[test]
    fn removes_csi_erase() {
        let result = sanitize_for_display("line\x1b[K");
        assert_eq!(result, "line");
    }

    #[test]
    fn removes_carriage_return() {
        let result = sanitize_for_display("foo\rbar");
        assert_eq!(result, "foobar");
    }

    #[test]
    fn preserves_newlines_and_tabs() {
        let result = sanitize_for_display("hello\n\tworld");
        assert_eq!(result, "hello\n\tworld");
    }

    #[test]
    fn removes_bell() {
        let result = sanitize_for_display("beep\x07");
        assert_eq!(result, "beep");
    }

    #[test]
    fn removes_cursor_hide_show() {
        let result = sanitize_for_display("\x1b[?25lhidden\x1b[?25h");
        assert_eq!(result, "hidden");
    }

    #[test]
    fn handles_complex_progress_bar() {
        // Typical tqdm output
        let input = "\rDownloading:  50%|████▌     | 5/10 [00:01<00:01,  4.99it/s]\x1b[K";
        let result = sanitize_for_display(input);
        assert_eq!(
            result,
            "Downloading:  50%|████▌     | 5/10 [00:01<00:01,  4.99it/s]"
        );
    }

    #[test]
    fn empty_string() {
        assert_eq!(sanitize_for_display(""), "");
    }

    #[test]
    fn no_control_chars() {
        assert_eq!(sanitize_for_display("Hello, world!"), "Hello, world!");
    }
}
