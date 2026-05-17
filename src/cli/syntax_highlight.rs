//! Simple Python syntax highlighter for terminal display.
//!
//! Uses a state machine to track triple-quoted strings across lines,
//! then applies ANSI colour codes to keywords, strings, comments,
//! numbers, and decorators.

use colored::Colorize;
use std::collections::HashSet;

/// Returns `code` with ANSI escape sequences that colour Python syntax.
pub fn highlight_python(code: &str) -> String {
    let keywords: HashSet<&str> = [
        "def", "class", "return", "if", "elif", "else", "for", "while", "import", "from", "as",
        "try", "except", "finally", "raise", "with", "yield", "lambda", "pass", "break",
        "continue", "and", "or", "not", "in", "is", "None", "True", "False", "async", "await",
        "global", "nonlocal", "assert", "del", "match", "case",
    ]
    .iter()
    .copied()
    .collect();

    let builtins: HashSet<&str> = [
        "print",
        "len",
        "range",
        "int",
        "str",
        "float",
        "list",
        "dict",
        "set",
        "tuple",
        "bool",
        "type",
        "open",
        "enumerate",
        "zip",
        "map",
        "filter",
        "sorted",
        "reversed",
        "any",
        "all",
        "sum",
        "min",
        "max",
        "abs",
        "round",
        "isinstance",
        "hasattr",
        "getattr",
        "setattr",
        "super",
        "self",
        "__init__",
    ]
    .iter()
    .copied()
    .collect();

    let mut result = String::with_capacity(code.len() + code.len() / 3);
    let mut in_triple_single = false;
    let mut in_triple_double = false;

    for line in code.lines() {
        // multi-line triple-quoted string continuation
        if in_triple_single {
            if let Some(pos) = line.find(triple_single_literal()) {
                result.push_str(&line[..pos + 3].bright_green().to_string());
                result.push('\n');
                in_triple_single = false;
            } else {
                result.push_str(&line.bright_green().to_string());
                result.push('\n');
            }
            continue;
        }
        if in_triple_double {
            if let Some(pos) = line.find(triple_double_literal()) {
                result.push_str(&line[..pos + 3].bright_green().to_string());
                result.push('\n');
                in_triple_double = false;
            } else {
                result.push_str(&line.bright_green().to_string());
                result.push('\n');
            }
            continue;
        }

        // normal line -- tokenise character-by-character
        let chars: Vec<char> = line.chars().collect();
        let len = chars.len();
        let mut i: usize = 0;

        while i < len {
            let c = chars[i];

            // Comment
            if c == '#' {
                result.push_str(&chars[i..].iter().collect::<String>().dimmed().to_string());
                break;
            }

            // String literals
            if c == '\'' || c == '"' {
                let quote = c;
                let mut j = i;

                // Triple-quote?
                if j + 2 < len && chars[j + 1] == quote && chars[j + 2] == quote {
                    j += 3;
                    let mut closed = false;
                    while j + 2 < len {
                        if chars[j] == quote && chars[j + 1] == quote && chars[j + 2] == quote {
                            j += 3;
                            closed = true;
                            break;
                        }
                        j += 1;
                    }
                    if !closed {
                        if quote == '\'' {
                            in_triple_single = true;
                        } else {
                            in_triple_double = true;
                        }
                        j = len;
                    }
                } else {
                    // regular single/double-quoted string
                    j += 1;
                    while j < len {
                        if chars[j] == '\\' && j + 1 < len {
                            j += 2;
                            continue;
                        }
                        if chars[j] == quote {
                            j += 1;
                            break;
                        }
                        j += 1;
                    }
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

            // Decorator
            if c == '@' && (i == 0 || chars[..i].iter().all(|ch| ch.is_whitespace())) {
                let j = word_end(&chars, i);
                result.push_str(
                    &chars[i..j]
                        .iter()
                        .collect::<String>()
                        .bright_red()
                        .to_string(),
                );
                i = j;
                continue;
            }

            // Number
            if c.is_ascii_digit() || (c == '.' && i + 1 < len && chars[i + 1].is_ascii_digit()) {
                let j = number_end(&chars, i);
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

            // Word (identifier / keyword / builtin)
            if c.is_alphabetic() || c == '_' {
                let j = word_end(&chars, i);
                let word: String = chars[i..j].iter().collect();
                if keywords.contains(word.as_str()) {
                    result.push_str(&word.bright_yellow().bold().to_string());
                } else if builtins.contains(word.as_str()) {
                    result.push_str(&word.bright_cyan().to_string());
                } else {
                    result.push_str(&word);
                }
                i = j;
                continue;
            }

            // Everything else -- pass through unchanged
            result.push(c);
            i += 1;
        }

        result.push('\n');
    }

    // Trim trailing newline we added onto the last line
    if result.ends_with('\n') && !code.ends_with('\n') {
        result.pop();
    }

    result
}

/// Returns the triple-single-quote literal: '''
fn triple_single_literal() -> &'static str {
    "'''"
}

/// Returns the triple-double-quote literal: """
fn triple_double_literal() -> &'static str {
    "\"\"\""
}

fn word_end(chars: &[char], start: usize) -> usize {
    let mut j = start;
    while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
        j += 1;
    }
    j
}

fn number_end(chars: &[char], start: usize) -> usize {
    let mut j = start;

    // hex / bin / oct prefix
    if j + 1 < chars.len() && chars[j] == '0' {
        match chars[j + 1] {
            'x' | 'X' | 'b' | 'B' | 'o' | 'O' => {
                j += 2;
                while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                return j;
            }
            _ => {}
        }
    }

    let mut seen_dot = false;
    while j < chars.len() {
        if chars[j].is_ascii_digit() || chars[j] == '_' {
            j += 1;
        } else if chars[j] == '.'
            && !seen_dot
            && j + 1 < chars.len()
            && chars[j + 1].is_ascii_digit()
        {
            seen_dot = true;
            j += 1;
        } else {
            break;
        }
    }
    j
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_highlight_keywords() {
        let code = "def foo():\n    return True\n";
        let result = highlight_python(code);
        assert!(result.contains("foo"));
    }

    #[test]
    fn test_highlight_triple_string_multiline() {
        let code = "s = '''\nhello\nworld\n'''\nx = 1\n";
        let result = highlight_python(code);
        assert!(result.contains("hello"));
        assert!(result.contains("x"));
    }

    #[test]
    fn test_triple_double_multiline() {
        let code = "s = \"\"\"\nhello\nworld\n\"\"\"\nx = 1\n";
        let result = highlight_python(code);
        assert!(result.contains("hello"));
        assert!(result.contains("x"));
    }

    #[test]
    fn test_multibyte_chars_no_panic() {
        // Regression: slicing with byte indices on multibyte chars panics
        let code = "print(f\"1から50までの合計: {total}\")  # 日本語\n";
        let result = highlight_python(code);
        assert!(result.contains("から"));
    }
}
