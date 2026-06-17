//! Shell command syntax highlighter.
//!
//! Provides syntax highlighting for shell commands displayed in the terminal.
//! All implementations are manual (no external syntax highlighting crates).
//!
//! Color scheme: **Catppuccin Mocha** inspired — the most popular color scheme
//! across editors and terminals.
//!
//! # Palette
//!
//! | Token               | Color          | Hex       |
//! |---------------------|----------------|-----------|
//! | Commands            | Yellow         | `#f9e2af` |
//! | Control keywords    | Mauve          | `#cba6f7` |
//! | Double-quoted strs  | Green          | `#a6e3a1` |
//! | Single-quoted strs  | Teal           | `#94e2d5` |
//! | Operators           | Peach/Orange   | `#fab387` |
//! | Redirections        | Blue           | `#89b4fa` |
//! | Variables           | Pink           | `#f5c2e7` |
//! | Comments            | Overlay0       | `#6c7086` |
//! | Backtick sub        | Sapphire       | `#74c7ec` |
//! | Options/Flags       | Subtext1       | `#bac2de` |
//! | Arithmetic exp      | Rosewater      | `#f5e0dc` |
//!
//! Design principles:
//! - High contrast & readability (視認性優先)
//! - No `.dimmed()` or gray-washed tones (グレー系不使用)
//! - Distinct from UI label colors (`cyan` is reserved for UI labels)
//! - Options/flags (`-la`, `--help`) highlighted to avoid blending into arguments

use colored::Colorize;

// ---------------------------------------------------------------------------
// Catppuccin Mocha palette (truecolor)
// ---------------------------------------------------------------------------
const CMD: (u8, u8, u8) = (249, 226, 175); // Yellow  — commands
const KEYWORD: (u8, u8, u8) = (203, 166, 247); // Mauve   — control keywords
const STR_DQ: (u8, u8, u8) = (166, 227, 161); // Green   — double-quoted strings
const STR_SQ: (u8, u8, u8) = (148, 226, 213); // Teal    — single-quoted strings
const OPERATOR: (u8, u8, u8) = (250, 179, 135); // Peach   — && || | ; &
const REDIRECT: (u8, u8, u8) = (137, 180, 250); // Blue    — > < >> 2>&1
const VARIABLE: (u8, u8, u8) = (245, 194, 231); // Pink    — $VAR ${} $(())
const COMMENT: (u8, u8, u8) = (108, 112, 134); // Overlay0 — # comments
const BACKTICK: (u8, u8, u8) = (116, 199, 236); // Sapphire — `cmd`
const OPTION: (u8, u8, u8) = (186, 194, 222); // Subtext1 — -la --help
const ARITH: (u8, u8, u8) = (245, 224, 220); // Rosewater — $(( ... ))

/// Apply a truecolor foreground to a string, then bold it.
macro_rules! paint {
    ($s:expr, $r:expr, $g:expr, $b:expr) => {
        $s.truecolor($r, $g, $b).bold().to_string()
    };
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

#[must_use]
pub fn highlight_shell_command(input: &str) -> String {
    let mut output = String::with_capacity(input.len() * 2);

    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // ── Line comments (# ...) ──────────────────────────────────────────
        if chars[i] == '#' && (i == 0 || chars[i - 1].is_whitespace()) {
            let start = i;
            while i < len && chars[i] != '\n' {
                i += 1;
            }
            let segment: String = chars[start..i].iter().collect();
            output.push_str(
                &segment
                    .truecolor(COMMENT.0, COMMENT.1, COMMENT.2)
                    .to_string(),
            );
            continue;
        }

        // ── Single-quoted strings ──────────────────────────────────────────
        if chars[i] == '\'' {
            let start = i;
            i += 1;
            while i < len && chars[i] != '\'' {
                if chars[i] == '\\' && i + 1 < len {
                    i += 1;
                }
                i += 1;
            }
            if i < len {
                i += 1;
            }
            let segment: String = chars[start..i].iter().collect();
            output.push_str(
                &segment
                    .truecolor(STR_SQ.0, STR_SQ.1, STR_SQ.2)
                    .bold()
                    .to_string(),
            );
            continue;
        }

        // ── Double-quoted strings ──────────────────────────────────────────
        if chars[i] == '"' {
            let start = i;
            i += 1;
            while i < len && chars[i] != '"' {
                if chars[i] == '\\' && i + 1 < len {
                    i += 1;
                }
                i += 1;
            }
            if i < len {
                i += 1;
            }
            let segment: String = chars[start..i].iter().collect();
            output.push_str(&segment.truecolor(STR_DQ.0, STR_DQ.1, STR_DQ.2).to_string());
            continue;
        }

        // ── $(( ... )) arithmetic expansion ────────────────────────────────
        if i + 2 < len && chars[i] == '$' && chars[i + 1] == '(' && chars[i + 2] == '(' {
            let start = i;
            i += 3;
            let mut depth = 1;
            while i + 1 < len && depth > 0 {
                if chars[i] == '(' && chars[i + 1] == '(' {
                    depth += 1;
                    i += 1;
                } else if chars[i] == ')' && chars[i + 1] == ')' {
                    depth -= 1;
                    i += 1;
                }
                i += 1;
            }
            if i < len {
                i += 1;
            }
            let segment: String = chars[start..i].iter().collect();
            output.push_str(&paint!(segment, ARITH.0, ARITH.1, ARITH.2));
            continue;
        }

        // ── $(...) command substitution ────────────────────────────────────
        if i + 1 < len && chars[i] == '$' && chars[i + 1] == '(' {
            let start = i;
            i += 2;
            let mut depth = 1;
            while i < len && depth > 0 {
                if chars[i] == '(' {
                    depth += 1;
                } else if chars[i] == ')' {
                    depth -= 1;
                } else if chars[i] == '\'' {
                    i += 1;
                    while i < len && chars[i] != '\'' {
                        if chars[i] == '\\' && i + 1 < len {
                            i += 1;
                        }
                        i += 1;
                    }
                } else if chars[i] == '"' {
                    i += 1;
                    while i < len && chars[i] != '"' {
                        if chars[i] == '\\' && i + 1 < len {
                            i += 1;
                        }
                        i += 1;
                    }
                }
                if depth > 0 {
                    i += 1;
                }
            }
            if i < len {
                i += 1;
            }
            let segment: String = chars[start..i].iter().collect();
            output.push_str(&paint!(segment, VARIABLE.0, VARIABLE.1, VARIABLE.2));
            continue;
        }

        // ── Backtick command substitution ──────────────────────────────────
        if chars[i] == '`' {
            let start = i;
            i += 1;
            while i < len && chars[i] != '`' {
                if chars[i] == '\\' && i + 1 < len {
                    i += 1;
                }
                i += 1;
            }
            if i < len {
                i += 1;
            }
            let segment: String = chars[start..i].iter().collect();
            output.push_str(
                &segment
                    .truecolor(BACKTICK.0, BACKTICK.1, BACKTICK.2)
                    .bold()
                    .to_string(),
            );
            continue;
        }

        // ── ${...} variable expansion ──────────────────────────────────────
        if i + 2 < len && chars[i] == '$' && chars[i + 1] == '{' {
            let start = i;
            i += 2;
            let mut depth = 1;
            while i < len && depth > 0 {
                if chars[i] == '{' {
                    depth += 1;
                } else if chars[i] == '}' {
                    depth -= 1;
                }
                i += 1;
            }
            let segment: String = chars[start..i].iter().collect();
            output.push_str(
                &segment
                    .truecolor(VARIABLE.0, VARIABLE.1, VARIABLE.2)
                    .bold()
                    .to_string(),
            );
            continue;
        }

        // ── $VAR (simple variable) ─────────────────────────────────────────
        if chars[i] == '$' && i + 1 < len && (chars[i + 1].is_alphanumeric() || chars[i + 1] == '_')
        {
            let start = i;
            i += 1;
            while i < len && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let segment: String = chars[start..i].iter().collect();
            output.push_str(
                &segment
                    .truecolor(VARIABLE.0, VARIABLE.1, VARIABLE.2)
                    .bold()
                    .to_string(),
            );
            continue;
        }

        // ── Special shell variables ($?, $#, $@, $*, $-, $$, $!, $0..$9) ──
        if chars[i] == '$' && i + 1 < len {
            let special = chars[i + 1];
            if "?#@*-!".contains(special) || special.is_ascii_digit() {
                let segment: String = chars[i..=i + 1].iter().collect();
                output.push_str(
                    &segment
                        .truecolor(VARIABLE.0, VARIABLE.1, VARIABLE.2)
                        .bold()
                        .to_string(),
                );
                i += 2;
                continue;
            }
        }

        // ── Multi-character operators/separators ───────────────────────────
        if i + 1 < len {
            let two_char: String = chars[i..=i + 1].iter().collect();
            if matches!(two_char.as_str(), "&&" | "||" | ";;" | "|&" | ";&" | ";;&") {
                output.push_str(&paint!(two_char, OPERATOR.0, OPERATOR.1, OPERATOR.2));
                i += 2;
                continue;
            }

            // Redirections with file descriptors (2>&1, 1>&2, etc.)
            if chars[i].is_ascii_digit() && i + 2 < len {
                let three_char: String = chars[i..=i + 2].iter().collect();
                if matches!(three_char.as_str(), "2>&" | "1>&" | "2>|" | "1>|") {
                    output.push_str(&paint!(three_char, REDIRECT.0, REDIRECT.1, REDIRECT.2));
                    i += 3;
                    continue;
                }
            }

            // Here-doc suffix <<-, <<<
            if i + 2 < len {
                let three_char: String = chars[i..=i + 2].iter().collect();
                if matches!(three_char.as_str(), "<<<" | "<<-") {
                    output.push_str(&paint!(three_char, REDIRECT.0, REDIRECT.1, REDIRECT.2));
                    i += 3;
                    continue;
                }
            }
        }

        // ── Single-char operators ──────────────────────────────────────────
        if matches!(chars[i], '|' | ';' | '&') {
            output.push_str(&paint!(
                chars[i].to_string(),
                OPERATOR.0,
                OPERATOR.1,
                OPERATOR.2
            ));
            i += 1;
            continue;
        }

        // ── Redirection operators ──────────────────────────────────────────
        if matches!(chars[i], '>' | '<') {
            if i + 1 < len {
                let two: String = chars[i..=i + 1].iter().collect();
                if matches!(two.as_str(), ">>" | "<<" | "<>" | "<&" | ">&" | ">|") {
                    output.push_str(&paint!(two, REDIRECT.0, REDIRECT.1, REDIRECT.2));
                    i += 2;
                    continue;
                }
            }
            output.push_str(&paint!(
                chars[i].to_string(),
                REDIRECT.0,
                REDIRECT.1,
                REDIRECT.2
            ));
            i += 1;
            continue;
        }

        // ── Words (commands, keywords, flags, etc.) ────────────────────────
        if chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '-' || chars[i] == '.' {
            let start = i;
            while i < len
                && (chars[i].is_alphanumeric()
                    || chars[i] == '_'
                    || chars[i] == '-'
                    || chars[i] == '.'
                    || chars[i] == '/')
            {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();

            if is_control_keyword(&word) {
                // Control keywords → Mauve (bold)
                output.push_str(&paint!(word, KEYWORD.0, KEYWORD.1, KEYWORD.2));
            } else if word.starts_with('-') || word.starts_with("--") {
                // Options/flags → Subtext1 (bold) — high visibility, distinct
                output.push_str(&paint!(word, OPTION.0, OPTION.1, OPTION.2));
            } else if is_command(&word) {
                // Commands → Yellow (bold)
                output.push_str(&paint!(word, CMD.0, CMD.1, CMD.2));
            } else if word.starts_with('/') || word.starts_with("./") || word.starts_with("../") {
                // Paths → Subtext1 (plain, no bold to avoid visual overload)
                output.push_str(&word.truecolor(OPTION.0, OPTION.1, OPTION.2).to_string());
            } else {
                // Plain word (arguments, filenames, etc.)
                output.push_str(&word);
            }

            continue;
        }

        // ── Default: pass through as-is ────────────────────────────────────
        output.push(chars[i]);
        i += 1;
    }

    output
}

// ---------------------------------------------------------------------------
// Keyword / command classification
// ---------------------------------------------------------------------------

/// Check if a word is a shell control flow keyword.
fn is_control_keyword(word: &str) -> bool {
    matches!(
        word,
        "if" | "then"
            | "else"
            | "elif"
            | "fi"
            | "for"
            | "while"
            | "until"
            | "do"
            | "done"
            | "case"
            | "esac"
            | "select"
            | "in"
            | "function"
            | "return"
            | "break"
            | "continue"
            | "!"
    )
}

/// Check if a word is a common shell command (builtin or well-known tool).
fn is_command(word: &str) -> bool {
    is_builtin(word) || is_common_command(word)
}

/// Check if a word is a shell builtin command.
fn is_builtin(word: &str) -> bool {
    matches!(
        word,
        "alias"
            | "bg"
            | "bind"
            | "builtin"
            | "caller"
            | "cd"
            | "command"
            | "declare"
            | "dirs"
            | "disown"
            | "echo"
            | "enable"
            | "eval"
            | "exec"
            | "exit"
            | "export"
            | "fc"
            | "fg"
            | "getopts"
            | "hash"
            | "help"
            | "history"
            | "jobs"
            | "kill"
            | "let"
            | "local"
            | "logout"
            | "popd"
            | "pushd"
            | "pwd"
            | "read"
            | "readonly"
            | "set"
            | "shift"
            | "shopt"
            | "source"
            | "suspend"
            | "test"
            | "times"
            | "trap"
            | "type"
            | "typeset"
            | "ulimit"
            | "umask"
            | "unalias"
            | "unset"
            | "wait"
            | "."
            | ":"
    )
}

/// Check if a word is a common external command.
fn is_common_command(word: &str) -> bool {
    matches!(
        word,
        // File system
        "ls" | "ll" | "la" | "find" | "locate" | "updatedb"
            | "cp" | "mv" | "rm" | "mkdir" | "rmdir" | "touch"
            | "ln" | "chmod" | "chown" | "chgrp" | "df" | "du"
            | "mount" | "umount" | "stat" | "tree" | "basename"
            | "dirname" | "realpath" | "mktemp"
            // Text processing
            | "cat" | "tac" | "less" | "more" | "head" | "tail"
            | "grep" | "egrep" | "fgrep" | "rg" | "ag" | "ack"
            | "sed" | "awk" | "cut" | "sort" | "uniq" | "wc"
            | "tr" | "diff" | "patch" | "cmp" | "comm"
            | "tee" | "fold" | "fmt" | "pr" | "nl" | "od"
            | "string" | "strings" | "rev" | "join" | "paste"
            | "column" | "expand" | "unexpand" | "split" | "csplit"
            | "iconv" | "dos2unix" | "unix2dos"
            | "jq" | "yq"
            // Compression / Archiving
            | "tar" | "gzip" | "gunzip" | "bzip2" | "bunzip2"
            | "xz" | "unxz" | "zstd" | "unzstd" | "lz4"
            | "zip" | "unzip" | "7z" | "rar" | "unrar"
            | "zcat" | "zless" | "zgrep" | "zdiff"
            | "compress" | "lzma" | "unlzma"
            // Process management
            | "ps" | "top" | "htop" | "btm" | "pgrep" | "pkill"
            | "pidof" | "kill" | "killall" | "nice" | "renice"
            | "nohup" | "timeout" | "watch" | "fuser" | "lsof"
            | "uptime" | "w" | "who" | "last"
            // Network
            | "curl" | "wget" | "httpie" | "http"
            | "ssh" | "scp" | "sftp" | "rsync"
            | "ping" | "traceroute" | "tracepath" | "mtr"
            | "netstat" | "ss" | "ip" | "ifconfig" | "iwconfig"
            | "dig" | "nslookup" | "host" | "nmap"
            | "nc" | "netcat" | "socat"
            | "iptables" | "ufw" | "firewall-cmd"
            | "telnet" | "ftp" | "smbclient"
            | "tcpdump" | "tshark" | "tcpflow"
            // Development / Build tools
            | "git" | "svn" | "hg" | "bzr"
            | "make" | "cmake" | "meson" | "ninja" | "just"
            | "cargo" | "rustc" | "rustup" | "clippy"
            | "python" | "python3" | "pip" | "pip3" | "uv"
            | "node" | "npm" | "npx" | "yarn" | "pnpm" | "bun"
            | "deno" | "bunx"
            | "ruby" | "gem" | "bundle" | "rake" | "rails"
            | "go" | "gofmt" | "golang"
            | "java" | "javac" | "gradle" | "mvn" | "mvnw"
            | "kotlin" | "scala" | "sbt"
            | "gcc" | "g++" | "clang" | "clang++"
            | "ld" | "as" | "ar" | "nm" | "objdump" | "readelf"
            | "perl" | "php" | "lua" | "luajit"
            | "swift" | "zig" | "nim" | "dart" | "flutter"
            | "racket" | "sbcl" | "ghc" | "cabal" | "stack"
            | "elixir" | "mix" | "erlc"
            | "haxe" | "julia" | "R" | "matlab" | "octave"
            | "wasm-pack" | "wasmtime" | "wasmer"
            // System administration
            | "sudo" | "doas" | "su" | "chroot" | "env" | "printenv"
            | "which" | "whereis" | "whatis" | "apropos" | "man"
            | "systemctl" | "journalctl" | "service"
            | "docker" | "podman" | "docker-compose" | "nerdctl"
            | "kubectl" | "minikube" | "helm"
            | "apt" | "apt-get" | "apt-cache" | "dpkg"
            | "yum" | "dnf" | "rpm" | "zypper" | "pacman" | "yay"
            | "brew" | "port" | "nix" | "flatpak" | "snap"
            | "cron" | "at" | "crontab"
            | "chsh" | "passwd" | "useradd" | "usermod" | "userdel"
            | "groupadd" | "groupmod" | "groupdel"
            // Files and content
            | "file" | "xxd" | "hexdump" | "hexyl" | "bat"
            | "fzf" | "fd" | "ripgrep"
            | "xargs" | "envsubst" | "stdbuf"
            // Screen / Terminal
            | "clear" | "reset" | "tput" | "stty"
            | "tmux" | "screen" | "byobu"
            | "echo" | "printf" | "yes" | "seq"
            // Date / Time
            | "date" | "cal" | "time" | "sleep"
            // Miscellaneous
            | "nproc" | "uname" | "hostname" | "arch"
            | "id" | "logname" | "groups" | "whoami"
            | "sh" | "bash" | "zsh" | "fish" | "ksh" | "dash" | "ash"
            | "nano" | "vim" | "vi" | "nvim" | "emacs" | "ed" | "ex"
            | "code" | "codium" | "zed"
            | "cc" | "c++"
            | "flex" | "bison" | "yacc" | "lex"
            | "pkg-config" | "autoconf" | "automake" | "libtool"
            | "install" | "strip" | "objcopy"
            | "openssl" | "gpg" | "age" | "sops" | "vault"
            | "cryptsetup" | "luks"
            | "dd" | "fdisk" | "parted" | "mkfs" | "fsck" | "blkid"
            | "free" | "vmstat" | "iostat" | "mpstat" | "sar"
            | "lscpu" | "lsblk" | "lspci" | "lsusb" | "lshw"
            | "dmesg" | "sysctl"
            | "screenfetch" | "neofetch" | "fastfetch"
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn force_color() {
        colored::control::set_override(true);
    }

    #[test]
    fn test_highlight_basic_command() {
        force_color();
        let result = highlight_shell_command("ls -la");
        assert!(
            result.contains("\x1b["),
            "Output should contain ANSI escape codes: {:?}",
            result
        );
        assert!(result.contains("ls"), "Should contain the command");
        assert!(result.contains("-la"), "Should contain the flag");
    }

    #[test]
    fn test_highlight_pipe_and_separators() {
        force_color();
        let result = highlight_shell_command("ls | grep foo && echo 'done'");
        assert!(result.contains("\x1b["), "Output should contain ANSI codes");
        assert!(result.contains("ls"), "Should contain ls");
        assert!(result.contains("&&"), "Should contain &&");
    }

    #[test]
    fn test_highlight_strings() {
        force_color();
        let result = highlight_shell_command("echo \"hello world\" 'single quoted'");
        assert!(result.contains("\x1b["), "Output should contain ANSI codes");
        assert!(result.contains("hello"), "Should contain hello");
        assert!(result.contains("single"), "Should contain single");
    }

    #[test]
    fn test_highlight_control_keywords() {
        force_color();
        let result = highlight_shell_command("if [ -f file ]; then echo exists; fi");
        assert!(result.contains("\x1b["), "Output should contain ANSI codes");
        assert!(result.contains("if"), "Should contain if");
    }

    #[test]
    fn test_highlight_comment() {
        force_color();
        let result = highlight_shell_command("echo foo # this is a comment");
        assert!(result.contains("\x1b["), "Output should contain ANSI codes");
        assert!(result.contains("comment"), "Should contain comment text");
    }

    #[test]
    fn test_highlight_redirections() {
        force_color();
        let result = highlight_shell_command("cat file.txt > /dev/null 2>&1");
        assert!(result.contains("\x1b["), "Output should contain ANSI codes");
        assert!(result.contains(">"), "Should contain >");
    }

    #[test]
    fn test_multiline_script() {
        force_color();
        let result = highlight_shell_command("for i in 1 2 3\ndo\n  echo \"$i\"\ndone");
        assert!(result.contains("\x1b["), "Output should contain ANSI codes");
    }

    #[test]
    fn test_empty_input() {
        assert_eq!(
            highlight_shell_command(""),
            "",
            "Empty input should produce empty output"
        );
    }

    #[test]
    fn test_no_special_chars() {
        let result = highlight_shell_command("just some plain text");
        assert!(!result.is_empty(), "Should return non-empty output");
    }

    #[test]
    fn test_escaped_quotes() {
        force_color();
        let result = highlight_shell_command("echo \"hello \\\"world\\\"\"");
        assert!(result.contains("\x1b["), "Output should contain ANSI codes");
    }

    #[test]
    fn test_arithmetic_expansion() {
        force_color();
        let result = highlight_shell_command("echo $((1 + 2))");
        assert!(
            result.contains("\x1b["),
            "Output should contain ANSI codes: {:?}",
            result
        );
    }

    #[test]
    fn test_flag_highlighting() {
        force_color();
        let result = highlight_shell_command("grep -r --include='*.rs' pattern");
        assert!(result.contains("-r"), "Should contain -r flag");
        assert!(
            result.contains("--include"),
            "Should contain --include flag"
        );
        assert!(result.contains("\x1b["), "Should have color");
    }

    #[test]
    fn test_here_doc() {
        force_color();
        let result = highlight_shell_command("cat <<< \"hello\"");
        assert!(result.contains("<<<"), "Should contain here-doc op");
    }
}
