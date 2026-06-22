use crate::cli::markdown::render_markdown;
use console::Term;

/// Print a block of content with optional title and style.
pub fn print_block(content: &str, title: Option<&str>, style: Option<&str>) {
    let term = Term::stdout();
    let (_, width) = term.size();
    let width = (width as usize).min(140);

    let mut output = String::new();

    if let Some(t) = title {
        let _rule_color = style.unwrap_or("cyan");
        let rule = "\u{2500}".repeat(width);
        output.push_str(&format!("{}\n", rule));
        output.push_str(&format!("{}\n", t));
    }

    // Use our custom markdown renderer
    let rendered = render_markdown(content.trim(), width);
    output.push_str(&rendered);
    output.push('\n');

    if title.is_some() {
        let _rule_color = style.unwrap_or("cyan");
        let rule = "\u{2500}".repeat(width);
        output.push_str(&format!("{}\n", rule));
    }
    print!("{output}");
}

/// Print a horizontal rule with optional title.
pub fn print_rule(title: Option<&str>, style: Option<&str>) {
    let term = Term::stdout();
    let (_, width) = term.size();
    let width = (width as usize).min(140);
    let _color = style.unwrap_or("cyan");

    if let Some(t) = title {
        let title_text = format!(" {t} ");
        let title_display = format!(" {} ", t);
        let text_width = console::measure_text_width(&title_text);
        let rule_len = width.saturating_sub(text_width);
        let left = 2;
        let right = rule_len.saturating_sub(left);
        println!(
            "{}{}{}",
            "\u{2500}".repeat(left),
            title_display,
            "\u{2500}".repeat(right)
        );
    } else {
        println!("{}", "\u{2500}".repeat(width));
    }
}

/// Print a key-value pair with formatting.
pub fn print_key_value(key: &str, value: &str) {
    println!("  {:15} {}", key, value);
}

/// Print a panel with content, title, and border style.
pub fn print_panel(
    content: &str,
    title: Option<&str>,
    _style: Option<&str>,
    border_style: Option<&str>,
) {
    let term = Term::stdout();
    let (_, term_width) = term.size();
    let width = (term_width as usize).clamp(40, 140);

    let _border_color = border_style.unwrap_or("cyan");

    // Top border
    if let Some(t) = title {
        let title_str = format!(" {} ", t);
        let remaining = width.saturating_sub(title_str.len() + 2);
        println!(
            "\u{2500}{title_str}\u{2500}{}",
            "\u{2500}".repeat(remaining)
        );
    } else {
        println!("{}", "\u{2500}".repeat(width));
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

    // Bottom border
    println!("{}", "\u{2500}".repeat(width));
}

/// Format a tool call display string (header + args + footer).
/// Returns the formatted string without printing.
pub fn format_tool_call(name: &str, args: &serde_json::Value, width: usize) -> String {
    let _color = "yellow";

    let mut buf = String::new();

    buf.push_str(&format!("{}\n", "\u{2500}".repeat(width)));
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

    buf.push_str(&format!("{}\n", "\u{2500}".repeat(width)));
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
    let _color = "green";
    let mut out = String::new();

    out.push_str(&format!(
        "  {}\n",
        "\u{2500}\u{2500} Result \u{2500}\u{2500}"
    ));

    // Sanitize the result string before displaying to prevent control characters
    // from corrupting the terminal display.
    let sanitized = crate::utils::sanitize_for_display(result);

    // Try to parse as JSON for pretty printing
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&sanitized) {
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
            let _status_color = if exit_code == 0 { "green" } else { "red" };

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
            push_line(&mut out, &format!("    {}", sanitized));
        }
    } else {
        for line in sanitized.lines() {
            push_line(&mut out, &format!("    {}", line));
        }
    }
    out.push('\n');
    finish_tool_result(out);
}

fn push_line(buf: &mut String, line: &str) {
    buf.push_str(line);
    buf.push('\n');
}

fn finish_tool_result(out: String) {
    print!("{out}");
    // Add separator line after tool result
    print_rule(None, None);
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
