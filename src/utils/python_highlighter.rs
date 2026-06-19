//! Python code syntax highlighter.
//!
//! Provides syntax highlighting for Python code displayed in the terminal.
//! All implementations are manual (no external syntax highlighting crates).
//!
//! Color scheme: **Basic ANSI colors** — optimized for broad terminal compatibility.
//!
//! # Palette
//!
//! | Token               | ANSI Color  |
//! |---------------------|-------------|
//! | Keywords            | Magenta     |
//! | Builtins            | Yellow      |
//! | Double-quoted strs  | Green       |
//! | Single-quoted strs  | Green       |
//! | Comments            | Green      |
//! | Decorators          | Cyan        |
//! | Numbers             | Cyan        |
//! | F-strings prefix    | Magenta     |
//! | Operators           | Red         |
//!
//! Design principles:
//! - High contrast & readability (視認性優先)
//! - Works well on both light and dark backgrounds
//! - No `.dimmed()` or gray-washed tones (グレー系不使用)
//! - Distinct from UI label colors (`cyan` is reserved for UI labels)

use colored::Colorize;

/// Apply a basic ANSI color foreground to a string, then bold it.
macro_rules! paint {
    ($s:expr, $color:ident) => {
        $s.$color().bold().to_string()
    };
}

// Python keywords
fn is_keyword(word: &str) -> bool {
    matches!(
        word,
        "False"
            | "None"
            | "True"
            | "and"
            | "as"
            | "assert"
            | "async"
            | "await"
            | "break"
            | "class"
            | "continue"
            | "def"
            | "del"
            | "elif"
            | "else"
            | "except"
            | "finally"
            | "for"
            | "from"
            | "global"
            | "if"
            | "import"
            | "in"
            | "is"
            | "lambda"
            | "nonlocal"
            | "not"
            | "or"
            | "pass"
            | "raise"
            | "return"
            | "try"
            | "while"
            | "with"
            | "yield"
            | "match"
            | "case"
            | "_"
    )
}

// Python built-in functions
fn is_builtin(word: &str) -> bool {
    matches!(
        word,
        "print" | "len" | "range" | "int" | "str" | "float"
            | "bool" | "list" | "dict" | "set" | "tuple"
            | "type" | "isinstance" | "issubclass"
            | "abs" | "all" | "any" | "bin" | "callable"
            | "chr" | "classmethod" | "compile" | "complex"
            | "delattr" | "dir" | "divmod" | "enumerate"
            | "eval" | "exec" | "filter" | "format"
            | "frozenset" | "getattr" | "globals" | "hasattr"
            | "hash" | "help" | "hex" | "id" | "input"
            | "iter" | "locals" | "map" | "max" | "min"
            | "next" | "object" | "oct" | "open" | "ord"
            | "pow" | "property" | "repr" | "reversed"
            | "round" | "setattr" | "slice" | "sorted"
            | "staticmethod" | "sum" | "super" | "vars"
            | "zip" | "__import__"
            // Common stdlib modules used as functions
            | "exit" | "quit"
    )
}

#[must_use]
pub fn highlight_python_code(input: &str) -> String {
    let mut output = String::with_capacity(input.len() * 2);

    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // ── Line comments (#) ──────────────────────────────────────────
        if chars[i] == '#' {
            let start = i;
            while i < len && chars[i] != '\n' {
                i += 1;
            }
            let segment: String = chars[start..i].iter().collect();
            output.push_str(&segment.green().to_string());
            continue;
        }

        // ── Triple-quoted strings (""" or ''') ─────────────────────────
        if (i + 2 < len)
            && ((chars[i] == '"' && chars[i + 1] == '"' && chars[i + 2] == '"')
                || (chars[i] == '\'' && chars[i + 1] == '\'' && chars[i + 2] == '\''))
        {
            let quote = chars[i];
            let start = i;
            i += 3;
            while i + 2 < len {
                if chars[i] == '\\' {
                    i += 2;
                    continue;
                }
                if chars[i] == quote && chars[i + 1] == quote && chars[i + 2] == quote {
                    i += 3;
                    break;
                }
                i += 1;
            }
            if i >= len - 2 {
                i = len;
            }
            let segment: String = chars[start..i].iter().collect();
            output.push_str(&segment.green().bold().to_string());
            continue;
        }

        // ── f-string prefix ────────────────────────────────────────────
        if (chars[i] == 'f' || chars[i] == 'F')
            && i + 1 < len
            && (chars[i + 1] == '"' || chars[i + 1] == '\'')
        {
            let f_prefix = chars[i].to_string();
            output.push_str(&paint!(f_prefix, magenta));
            i += 1;
            continue;
        }

        // ── Single-quoted strings ──────────────────────────────────────
        if chars[i] == '\'' {
            let start = i;
            i += 1;
            while i < len && chars[i] != '\'' {
                if chars[i] == '\\' && i + 1 < len {
                    i += 2;
                    continue;
                }
                i += 1;
            }
            if i < len {
                i += 1;
            }
            let segment: String = chars[start..i].iter().collect();
            output.push_str(&segment.green().bold().to_string());
            continue;
        }

        // ── Double-quoted strings ──────────────────────────────────────
        if chars[i] == '"' {
            let start = i;
            i += 1;
            while i < len && chars[i] != '"' {
                if chars[i] == '\\' && i + 1 < len {
                    i += 2;
                    continue;
                }
                i += 1;
            }
            if i < len {
                i += 1;
            }
            let segment: String = chars[start..i].iter().collect();
            output.push_str(&segment.green().to_string());
            continue;
        }

        // ── Decorators (@decorator) ────────────────────────────────────
        if chars[i] == '@' && i + 1 < len && (chars[i + 1].is_alphabetic() || chars[i + 1] == '_') {
            let start = i;
            i += 1;
            while i < len && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '.') {
                i += 1;
            }
            let segment: String = chars[start..i].iter().collect();
            output.push_str(&paint!(segment, cyan));
            continue;
        }

        // ── Numbers (integers and floats) ──────────────────────────────
        if chars[i].is_ascii_digit()
            || (chars[i] == '.' && i + 1 < len && chars[i + 1].is_ascii_digit())
        {
            let start = i;
            if chars[i] == '0'
                && i + 1 < len
                && (chars[i + 1] == 'x'
                    || chars[i + 1] == 'X'
                    || chars[i + 1] == 'b'
                    || chars[i + 1] == 'B'
                    || chars[i + 1] == 'o'
                    || chars[i + 1] == 'O')
            {
                i += 2;
                while i < len && (chars[i].is_ascii_hexdigit() || chars[i] == '_') {
                    i += 1;
                }
                let segment: String = chars[start..i].iter().collect();
                output.push_str(&paint!(segment, cyan));
                continue;
            }
            while i < len
                && (chars[i].is_ascii_digit()
                    || chars[i] == '.'
                    || chars[i] == '_'
                    || chars[i] == 'e'
                    || chars[i] == 'E'
                    || chars[i] == '+'
                    || chars[i] == '-')
            {
                // Hex/octal/binary prefix inside number
                if chars[i] == '+' || chars[i] == '-' {
                    if i > start && (chars[i - 1] == 'e' || chars[i - 1] == 'E') {
                        i += 1;
                        continue;
                    }
                    break;
                }
                i += 1;
            }
            if i > start {
                let segment: String = chars[start..i].iter().collect();
                // Only color if it's a valid number-like token
                if segment.chars().any(|c| c.is_ascii_digit()) {
                    output.push_str(&paint!(segment, cyan));
                    continue;
                }
                // Otherwise fall through to word handling
                i = start;
            }
        }

        // ── Words (keywords, builtins, identifiers) ───────────────────
        if chars[i].is_alphabetic() || chars[i] == '_' {
            let start = i;
            while i < len && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();

            if is_keyword(&word) {
                output.push_str(&paint!(word, magenta));
            } else if is_builtin(&word) {
                output.push_str(&paint!(word, yellow));
            } else {
                // Regular identifier — pass through
                output.push_str(&word);
            }
            continue;
        }

        // ── Operators ─────────────────────────────────────────────────
        if matches!(
            chars[i],
            '+' | '-' | '*' | '/' | '%' | '=' | '!' | '>' | '<' | '&' | '|' | '^' | '~' | '@'
        ) {
            let start = i;
            i += 1;
            // Consume multi-char operators
            while i < len
                && matches!(
                    chars[i],
                    '=' | '+' | '-' | '*' | '/' | '>' | '<' | '&' | '|' | '^'
                )
            {
                i += 1;
            }
            let segment: String = chars[start..i].iter().collect();
            output.push_str(&segment.red().to_string());
            continue;
        }

        // ── Default: pass through as-is ────────────────────────────────
        output.push(chars[i]);
        i += 1;
    }

    output
}

// Tests

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn force_color() {
        colored::control::set_override(true);
    }

    #[test]
    fn test_highlight_basic_print() {
        force_color();
        let result = highlight_python_code("print('hello world')");
        assert!(
            result.contains("\x1b["),
            "Output should contain ANSI escape codes"
        );
        assert!(result.contains("print"), "Should contain print");
        assert!(result.contains("hello"), "Should contain hello");
    }

    #[test]
    fn test_highlight_keyword() {
        force_color();
        let result = highlight_python_code("if x > 0:");
        assert!(result.contains("\x1b["), "Output should contain ANSI codes");
        assert!(result.contains("if"), "Should contain if");
    }

    #[test]
    fn test_highlight_import() {
        force_color();
        let result = highlight_python_code("import os");
        assert!(result.contains("\x1b["), "Output should contain ANSI codes");
        assert!(result.contains("import"), "Should contain import");
    }

    #[test]
    fn test_highlight_decorator() {
        force_color();
        let result = highlight_python_code("@staticmethod\ndef foo():");
        assert!(result.contains("\x1b["), "Output should contain ANSI codes");
        assert!(result.contains("@staticmethod"), "Should contain decorator");
    }

    #[test]
    fn test_highlight_comment() {
        force_color();
        let result = highlight_python_code("x = 1  # this is a comment");
        assert!(result.contains("\x1b["), "Output should contain ANSI codes");
        assert!(result.contains("comment"), "Should contain comment text");
    }

    #[test]
    fn test_highlight_number() {
        force_color();
        let result = highlight_python_code("x = 42");
        assert!(result.contains("\x1b["), "Output should contain ANSI codes");
        assert!(result.contains("42"), "Should contain 42");
    }

    #[test]
    fn test_highlight_f_string() {
        force_color();
        let result = highlight_python_code("f\"hello {name}\"");
        assert!(result.contains("\x1b["), "Output should contain ANSI codes");
        assert!(result.contains("hello"), "Should contain hello");
    }

    #[test]
    fn test_highlight_class_def() {
        force_color();
        let result = highlight_python_code("class MyClass:");
        assert!(result.contains("\x1b["), "Output should contain ANSI codes");
        assert!(result.contains("class"), "Should contain class keyword");
        assert!(result.contains("MyClass"), "Should contain class name");
    }

    #[test]
    fn test_highlight_function_def() {
        force_color();
        let result = highlight_python_code("def my_function():");
        assert!(result.contains("\x1b["), "Output should contain ANSI codes");
        assert!(result.contains("def"), "Should contain def keyword");
        assert!(
            result.contains("my_function"),
            "Should contain function name"
        );
    }

    #[test]
    fn test_empty_input() {
        assert_eq!(
            highlight_python_code(""),
            "",
            "Empty input should produce empty output"
        );
    }

    #[test]
    fn test_multiline_code() {
        force_color();
        let code = "for i in range(10):\n    print(i)";
        let result = highlight_python_code(code);
        assert!(result.contains("\x1b["), "Output should contain ANSI codes");
        assert!(result.contains("for"), "Should contain for");
        assert!(result.contains("range"), "Should contain range");
        assert!(result.contains("print"), "Should contain print");
    }

    #[test]
    fn test_triple_quoted_string() {
        force_color();
        let result = highlight_python_code("'''docstring'''");
        assert!(result.contains("\x1b["), "Output should contain ANSI codes");
        assert!(result.contains("docstring"), "Should contain docstring");
    }

    #[test]
    fn test_escape_sequences() {
        force_color();
        let result = highlight_python_code("print(\"hello \\\"world\\\"\")");
        assert!(result.contains("\x1b["), "Output should contain ANSI codes");
    }

    #[test]
    fn test_hex_number() {
        force_color();
        let result = highlight_python_code("x = 0xFF");
        assert!(result.contains("\x1b["), "Output should contain ANSI codes");
        assert!(result.contains("0xFF"), "Should contain hex number");
    }
}
