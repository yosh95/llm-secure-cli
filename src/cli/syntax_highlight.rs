//! Simple shell syntax highlighter for terminal display.
//!
//! Uses a state machine to track quoted strings across lines,
//! then applies ANSI colour codes to commands, flags, variables,
//! strings, comments, and numbers.

use colored::Colorize;
use std::collections::HashSet;

/// Returns `code` with ANSI escape sequences that colour shell syntax.
#[must_use]
pub fn highlight_shell(code: &str) -> String {
    let commands: HashSet<&str> = [
        "curl",
        "wget",
        "ls",
        "cd",
        "cat",
        "grep",
        "find",
        "echo",
        "git",
        "docker",
        "sudo",
        "pip",
        "npm",
        "cargo",
        "ps",
        "kill",
        "which",
        "chmod",
        "mkdir",
        "rm",
        "cp",
        "mv",
        "touch",
        "head",
        "tail",
        "sort",
        "uniq",
        "wc",
        "tee",
        "xargs",
        "sed",
        "awk",
        "env",
        "export",
        "alias",
        "type",
        "file",
        "stat",
        "du",
        "df",
        "make",
        "cmake",
        "gcc",
        "clang",
        "rustc",
        "go",
        "node",
        "deno",
        "bun",
        "npx",
        "yarn",
        "python3",
        "python",
        "ruby",
        "perl",
        "php",
        "systemctl",
        "journalctl",
        "service",
        "apt",
        "yum",
        "dnf",
        "ping",
        "ssh",
        "scp",
        "rsync",
        "tar",
        "gzip",
        "gunzip",
        "set",
        "shopt",
        "test",
        "let",
        "exit",
        "return",
        "if",
        "then",
        "else",
        "elif",
        "fi",
        "for",
        "while",
        "do",
        "done",
        "case",
        "esac",
        "in",
        "function",
        "select",
        "until",
    ]
    .iter()
    .copied()
    .collect();

    let mut result = String::with_capacity(code.len() + code.len() / 3);

    for line in code.lines() {
        let chars: Vec<char> = line.chars().collect();
        let len = chars.len();
        let mut i: usize = 0;

        while i < len {
            let c = chars[i];

            // Comment (#)
            if c == '#' {
                result.push_str(&chars[i..].iter().collect::<String>().dimmed().to_string());
                break;
            }

            // Single-quoted string: '...'  — literal, no escaping
            if c == '\'' {
                let mut j = i + 1;
                while j < len && chars[j] != '\'' {
                    j += 1;
                }
                if j < len {
                    j += 1; // closing quote
                }
                result.push_str(
                    &chars[i..j]
                        .iter()
                        .collect::<String>()
                        .bright_green()
                        .to_string(),
                );
                i = j;
                continue;
            }

            // Double-quoted string: "..." — supports $, \, ``
            if c == '"' {
                let mut j = i + 1;
                while j < len {
                    if chars[j] == '\\' && j + 1 < len {
                        j += 2;
                        continue;
                    }
                    if chars[j] == '"' {
                        j += 1;
                        break;
                    }
                    j += 1;
                }
                result.push_str(
                    &chars[i..j.min(len)]
                        .iter()
                        .collect::<String>()
                        .bright_green()
                        .to_string(),
                );
                i = j;
                continue;
            }

            // Backtick: `...` — command substitution
            if c == '`' {
                let mut j = i + 1;
                while j < len && chars[j] != '`' {
                    j += 1;
                }
                if j < len {
                    j += 1;
                }
                result.push_str(
                    &chars[i..j]
                        .iter()
                        .collect::<String>()
                        .bright_magenta()
                        .to_string(),
                );
                i = j;
                continue;
            }

            // Variable: $VAR, ${VAR}, $(...)
            if c == '$' {
                let mut j = i + 1;
                if j < len && chars[j] == '{' {
                    // ${...}
                    j += 1;
                    while j < len && chars[j] != '}' {
                        j += 1;
                    }
                    if j < len {
                        j += 1;
                    }
                } else if j < len && chars[j] == '(' {
                    // $(...) — command substitution
                    let mut depth = 1;
                    j += 1;
                    while j < len && depth > 0 {
                        if chars[j] == '(' {
                            depth += 1;
                        } else if chars[j] == ')' {
                            depth -= 1;
                        }
                        j += 1;
                    }
                } else {
                    // $VAR
                    while j < len && (chars[j].is_alphanumeric() || chars[j] == '_') {
                        j += 1;
                    }
                }
                result.push_str(
                    &chars[i..j]
                        .iter()
                        .collect::<String>()
                        .bright_magenta()
                        .to_string(),
                );
                i = j;
                continue;
            }

            // Number
            if c.is_ascii_digit() {
                let mut j = i;
                while j < len && (chars[j].is_ascii_digit() || chars[j] == '.' || chars[j] == '_') {
                    j += 1;
                }
                result.push_str(
                    &chars[i..j]
                        .iter()
                        .collect::<String>()
                        .bright_magenta()
                        .to_string(),
                );
                i = j;
                continue;
            }

            // Word (command or flag)
            if c.is_alphabetic() || c == '_' || c == '-' {
                let mut j = i;
                while j < len && (chars[j].is_alphanumeric() || chars[j] == '_' || chars[j] == '-')
                {
                    j += 1;
                }
                let word: String = chars[i..j].iter().collect();

                // Check if this is the first word (command name)
                let is_first_word = i == 0
                    || chars[..i]
                        .iter()
                        .all(|ch| ch.is_whitespace() || *ch == '|' || *ch == ';' || *ch == '&');

                if is_first_word && commands.contains(word.as_str()) {
                    result.push_str(&word.bright_cyan().bold().to_string());
                } else if word.starts_with('-') {
                    // Flags: -x, --option
                    result.push_str(&word.yellow().to_string());
                } else {
                    result.push_str(&word);
                }
                i = j;
                continue;
            }

            // Operators: |, >, <, &, ;
            if c == '|' || c == '>' || c == '<' || c == '&' || c == ';' {
                result.push_str(&c.to_string().bright_black().to_string());
                i += 1;
                continue;
            }

            // Everything else
            result.push(c);
            i += 1;
        }

        result.push('\n');
    }

    // Trim trailing newline
    if result.ends_with('\n') && !code.ends_with('\n') {
        result.pop();
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_highlight_command() {
        let code = "curl https://example.com";
        let result = highlight_shell(code);
        assert!(result.contains("curl"));
    }

    #[test]
    fn test_highlight_comment() {
        let code = "ls -la  # list files";
        let result = highlight_shell(code);
        assert!(result.contains("list files"));
    }

    #[test]
    fn test_highlight_variable() {
        let code = "echo $HOME";
        let result = highlight_shell(code);
        assert!(result.contains("HOME"));
    }

    #[test]
    fn test_highlight_flag() {
        let code = "ls -la --color=auto";
        let result = highlight_shell(code);
        assert!(result.contains("--color"));
    }
}
