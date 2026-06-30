use crate::cli::markdown::render_markdown;
use console::Term;

/// Print a block of content with optional title.
pub fn print_block(content: &str, title: Option<&str>) {
    let term = Term::stdout();
    let (_, width) = term.size();
    let width = (width as usize).min(140);

    let mut output = String::new();

    if let Some(t) = title {
        output.push_str(&format!("{}\n", t));
    }

    // Use our custom markdown renderer
    let rendered = render_markdown(content.trim(), width);
    output.push_str(&rendered);
    output.push('\n');

    print!("{output}");
}

/// Print a key-value pair with formatting.
pub fn print_key_value(key: &str, value: &str) {
    println!("  {:15} {}", key, value);
}

/// Print a panel with content and optional title (borders removed).
pub fn print_panel(content: &str, title: Option<&str>) {
    let term = Term::stdout();
    let (_, term_width) = term.size();
    let width = (term_width as usize).clamp(40, 140);

    // Title (if any)
    if let Some(t) = title {
        println!("{t}");
    }

    // Content with wrapping
    let inner_width = width - 4;
    let options = textwrap::Options::new(inner_width)
        .break_words(false)
        .word_splitter(textwrap::WordSplitter::NoHyphenation);

    for line in content.lines() {
        let wrapped = textwrap::wrap(line, &options);
        for w_line in wrapped {
            println!("    {w_line}");
        }
    }
}

/// Format a tool call display string (header + args).
/// Returns the formatted string without printing.
pub fn format_tool_call(name: &str, args: &serde_json::Value, _width: usize) -> String {
    let mut buf = String::new();

    buf.push_str(&format!("{} {}{}\n", "->", name, ":"));

    if let Some(obj) = args.as_object() {
        let explanation = obj.get("explanation").and_then(|v| v.as_str());

        let path_like = [
            "path",
            "url",
            "directory",
            "query",
            "pattern",
            "file_pattern",
            "content",
            "code",
            "old",
            "new",
            "ignore_patterns",
            "exclude_patterns",
            "include_hidden",
            "max_files",
            "depth",
        ];
        let start_keys = ["start_line"];
        let end_keys = ["end_line"];

        let mut remaining_keys: Vec<&String> = Vec::new();
        for k in obj.keys() {
            if k != "explanation" {
                remaining_keys.push(k);
            }
        }

        remaining_keys.sort_by(|a, b| {
            let a_is_path = path_like.contains(&a.as_str());
            let b_is_path = path_like.contains(&b.as_str());
            let a_is_start = start_keys.contains(&a.as_str());
            let b_is_start = start_keys.contains(&b.as_str());
            let a_is_end = end_keys.contains(&a.as_str());
            let b_is_end = end_keys.contains(&b.as_str());

            if a_is_path && !b_is_path {
                std::cmp::Ordering::Less
            } else if !a_is_path && b_is_path {
                std::cmp::Ordering::Greater
            } else if a_is_start && !b_is_start {
                std::cmp::Ordering::Less
            } else if !a_is_start && b_is_start {
                std::cmp::Ordering::Greater
            } else if a_is_end && !b_is_end {
                std::cmp::Ordering::Less
            } else if !a_is_end && b_is_end {
                std::cmp::Ordering::Greater
            } else {
                a.cmp(b)
            }
        });

        for k in remaining_keys {
            if let Some(v) = obj.get(k) {
                if name == "execute_python" && k == "code" {
                    // Display Python code with syntax highlighting
                    push_line(&mut buf, &format!("    {} {}:", "\u{2022}", "code"));
                    let code_str = v.as_str().unwrap_or("");
                    for line in code_str.lines() {
                        push_line(&mut buf, &format!("        {line}"));
                    }
                } else {
                    let val_str = v
                        .as_str()
                        .map_or_else(|| v.to_string(), std::string::ToString::to_string);
                    push_line(&mut buf, &format!("    {} {}: {}", "\u{2022}", k, val_str));
                }
            }
        }

        if let Some(exp) = explanation {
            let val_str = exp.to_string();
            push_line(
                &mut buf,
                &format!("    {} {}: {}", "\u{2022}", "explanation", val_str),
            );
        }
    } else {
        push_line(&mut buf, &format!("    {args}"));
    }
    buf.push('\n');
    buf
}

/// Print a tool call with formatting.
pub fn print_tool_call(name: &str, args: &serde_json::Value) {
    let term = Term::stdout();
    let (_, width) = term.size();
    let width = (width as usize).min(140);
    let buf = format_tool_call(name, args, width);
    print!("{buf}");
}

/// Print a tool call directly without formatting.
pub fn print_tool_call_direct(name: &str, args: &serde_json::Value) {
    let term = Term::stdout();
    let (_, width) = term.size();
    let width = (width as usize).min(140);
    let buf = format_tool_call(name, args, width);
    print!("{buf}");
}

pub fn print_tool_result(result: &str) {
    let mut out = String::new();

    out.push_str("  Result:\n");

    let result_str = result.to_string();

    // Try to parse as JSON for pretty printing
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&result_str) {
        // Special handling for file modification results
        if let (Some(path), Some(diff)) = (
            v.get("path").and_then(|v| v.as_str()),
            v.get("diff").and_then(|v| v.as_str()),
        ) {
            let message = v
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if !message.is_empty() {
                push_line(&mut out, &format!("    {} {}", "\u{2022}", message));
            }
            push_line(
                &mut out,
                &format!("    {} {}: {}", "\u{2022}", "path", path),
            );

            if !diff.is_empty() {
                push_line(&mut out, &format!("    {} {}:", "\u{2022}", "diff"));
                for line in diff.lines() {
                    push_line(&mut out, &format!("        {}", line));
                }
            }
            finish_tool_result(out);
            return;
        }

        // Special handling for command execution results
        if let (Some(stdout), Some(stderr), Some(exit_code)) = (
            v.get("stdout").and_then(|v| v.as_str()),
            v.get("stderr").and_then(|v| v.as_str()),
            v.get("exit_code").and_then(serde_json::Value::as_i64),
        ) {
            // Do not re-display stdout/stderr if already displayed in real-time by the tool side
            let is_real_time_displayed = v
                .get("_real_time_displayed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            if !is_real_time_displayed {
                if !stdout.is_empty() {
                    push_line(&mut out, &format!("    {}:", "STDOUT"));
                    for line in stdout.lines() {
                        push_line(&mut out, &format!("      {}", line));
                    }
                }

                if !stderr.is_empty() {
                    push_line(&mut out, &format!("    {}:", "STDERR"));
                    for line in stderr.lines() {
                        push_line(&mut out, &format!("      {}", line));
                    }
                }
            }

            push_line(&mut out, &format!("    {} {}", "Exit Code:", exit_code));

            // Display the note field if present (e.g., timeout or Ctrl+C message)
            if let Some(note) = v.get("note").and_then(|v| v.as_str())
                && !note.is_empty()
            {
                push_line(&mut out, &format!("    {} {}", "Note:", note));
            }

            finish_tool_result(out);
            return;
        }

        // Special handling for "matches" or "results" arrays
        for key in ["matches", "results", "files"] {
            if let Some(arr) = v.get(key).and_then(|a| a.as_array()) {
                if arr.is_empty() {
                    push_line(&mut out, &format!("    {}", "(empty results)"));
                } else {
                    for item in arr {
                        if let Some(s) = item.as_str() {
                            push_line(&mut out, &format!("    {} {}", "\u{2022}", s));
                        } else if let Some(obj) = item.as_object() {
                            if let (Some(file), Some(line), Some(text)) = (
                                obj.get("file").and_then(|v| v.as_str()),
                                obj.get("line"),
                                obj.get("text").and_then(|v| v.as_str()),
                            ) {
                                push_line(
                                    &mut out,
                                    &format!("    {} {}:{}: {}", "\u{2022}", file, line, text),
                                );
                            } else if let (Some(t), Some(path)) = (
                                obj.get("type").and_then(|v| v.as_str()),
                                obj.get("path").and_then(|v| v.as_str()),
                            ) {
                                let mut details = Vec::new();
                                if let Some(size) =
                                    obj.get("size").and_then(serde_json::Value::as_u64)
                                {
                                    details.push(format_size_brief(size));
                                }
                                if let Some(mtime) =
                                    obj.get("last_modified").and_then(|v| v.as_str())
                                {
                                    details.push(mtime.to_string());
                                }

                                if details.is_empty() {
                                    push_line(
                                        &mut out,
                                        &format!("    {} [{}] {}", "\u{2022}", t, path),
                                    );
                                } else {
                                    push_line(
                                        &mut out,
                                        &format!(
                                            "    {} [{}] {:<30}  {}",
                                            "\u{2022}",
                                            t,
                                            path,
                                            details.join(" | ")
                                        ),
                                    );
                                }
                            } else {
                                push_line(&mut out, &format!("    {} {}", "\u{2022}", item));
                            }
                        }
                    }
                }
                if let Some(truncated) = v.get("truncated").and_then(serde_json::Value::as_bool)
                    && truncated
                {
                    push_line(&mut out, &format!("    {}", "... (results truncated)"));
                }
                finish_tool_result(out);
                return;
            }
        }

        if let Ok(pretty) = serde_json::to_string_pretty(&v) {
            if v.is_object() || v.is_array() {
                for line in pretty.lines() {
                    push_line(&mut out, &format!("    {}", line));
                }
            } else if let Some(s) = v.as_str() {
                for line in s.lines() {
                    push_line(&mut out, &format!("    {}", line));
                }
            } else {
                push_line(&mut out, &format!("    {}", pretty));
            }
        } else {
            push_line(&mut out, &format!("    {}", result_str));
        }
    } else {
        for line in result_str.lines() {
            push_line(&mut out, &format!("    {}", line));
        }
    }
    out.push('\n');
    finish_tool_result(out);
}

/// Print a horizontal rule spanning the full terminal width.
pub fn print_rule() {
    let term = console::Term::stdout();
    let (_, width) = term.size();
    let width = (width as usize).clamp(10, 200);
    let rule = "─".repeat(width);
    println!("{rule}");
}

fn push_line(buf: &mut String, line: &str) {
    buf.push_str(line);
    buf.push('\n');
}

fn finish_tool_result(out: String) {
    println!("{out}");
}

fn format_size_brief(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes}B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1}GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}
