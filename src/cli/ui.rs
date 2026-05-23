use crate::cli::markdown::render_markdown;
use colored::*;
use console::Term;
use std::io::{Read, Write};

use async_trait::async_trait;

#[async_trait]
pub trait UserInterface: Send + Sync {
    fn print_block(&self, content: &str, title: Option<&str>, style: Option<&str>);
    fn print_rule(&self, title: Option<&str>, style: Option<&str>);
    fn print_tool_call(&self, name: &str, args: &serde_json::Value);
    fn print_tool_result(&self, result: &str);
    fn report_error(&self, message: &str);
    fn report_info(&self, message: &str);
    fn report_warning(&self, message: &str);
    fn report_success(&self, message: &str);
    async fn ask_confirm(&self, prompt: &str) -> Option<ConfirmResult>;
    async fn ask_confirm_simple(&self, prompt: &str) -> Option<ConfirmResult>;
}

pub struct CliUi;

#[async_trait]
impl UserInterface for CliUi {
    fn print_block(&self, content: &str, title: Option<&str>, style: Option<&str>) {
        print_block(content, title, style);
    }
    fn print_rule(&self, title: Option<&str>, style: Option<&str>) {
        print_rule(title, style);
    }
    fn print_tool_call(&self, name: &str, args: &serde_json::Value) {
        print_tool_call(name, args);
    }
    fn print_tool_result(&self, result: &str) {
        print_tool_result(result);
    }
    fn report_error(&self, message: &str) {
        report_error(message);
    }
    fn report_info(&self, message: &str) {
        report_info(message);
    }
    fn report_warning(&self, message: &str) {
        report_warning(message);
    }
    fn report_success(&self, message: &str) {
        report_success(message);
    }
    async fn ask_confirm(&self, prompt: &str) -> Option<ConfirmResult> {
        ask_confirm_async(prompt).await
    }
    async fn ask_confirm_simple(&self, prompt: &str) -> Option<ConfirmResult> {
        ask_confirm_simple_async(prompt).await
    }
}

pub fn print_block(content: &str, title: Option<&str>, style: Option<&str>) {
    let term = Term::stdout();
    let (_, width) = term.size();
    let width = (width as usize).min(140);

    let mut output = String::new();

    if let Some(t) = title {
        let rule_color = style.unwrap_or("cyan");
        let rule = "\u{2500}".repeat(width);
        output.push_str(&format!("{}\n", rule.color(rule_color)));
        output.push_str(&format!("{}\n", t.bold().color(rule_color)));
    }

    // Use our custom markdown renderer
    let rendered = render_markdown(content.trim(), width);
    output.push_str(&rendered);
    output.push('\n');

    if title.is_some() {
        let rule_color = style.unwrap_or("cyan");
        let rule = "\u{2500}".repeat(width);
        output.push_str(&format!("{}\n", rule.color(rule_color)));
    }

    let term_height = term.size().0;
    crate::cli::pager::page_output(&output, term_height);
}

pub fn print_rule(title: Option<&str>, style: Option<&str>) {
    let term = Term::stdout();
    let (_, width) = term.size();
    let width = (width as usize).min(140);
    let color = style.unwrap_or("bright_black");

    if let Some(t) = title {
        let title_text = format!(" {} ", t);
        let title_display = format!(" {} ", t.bold());
        let text_width = console::measure_text_width(&title_text);
        let rule_len = width.saturating_sub(text_width);
        let left = 2; // Fixed left margin for a more modern look
        let right = rule_len.saturating_sub(left);
        println!(
            "{}{}{}",
            "\u{2500}".repeat(left).color(color),
            title_display.color(color),
            "\u{2500}".repeat(right).color(color)
        );
    } else {
        println!("{}", "\u{2500}".repeat(width).color(color));
    }
}

pub fn print_key_value(key: &str, value: &str) {
    println!("  {:15} {}", key.bold().cyan(), value);
}

pub fn print_tool_call(name: &str, args: &serde_json::Value) {
    let term = Term::stdout();
    let (term_height, width) = term.size();
    let width = (width as usize).min(140);
    let color = "yellow";

    let mut buf = String::new();

    buf.push_str(&format!("{}\n", "\u{2500}".repeat(width).color(color)));
    buf.push_str(&format!(
        "{} {}{}\n",
        "->".yellow().bold(),
        name.bold().yellow(),
        ":".yellow()
    ));

    if let Some(obj) = args.as_object() {
        if name == "execute_python" {
            let code = obj.get("code").and_then(|v| v.as_str()).unwrap_or_default();
            let explanation = obj.get("explanation").and_then(|v| v.as_str());

            // Print explanation FIRST
            if let Some(exp) = explanation {
                push_line(
                    &mut buf,
                    &format!(
                        "    {} {}: {}",
                        "\u{2022}".bright_black(),
                        "explanation".cyan(),
                        exp
                    ),
                );
            }

            // Then print code (path-like)
            push_line(
                &mut buf,
                &format!("    {} {}:", "\u{2022}".bright_black(), "code".cyan()),
            );
            // Apply Python syntax highlighting and indent each line
            let highlighted = crate::cli::syntax_highlight::highlight_python(code);
            for line in highlighted.lines() {
                push_line(&mut buf, &format!("        {line}"));
            }

            // Print other arguments if any (except code and explanation)
            for (k, v) in obj {
                if k != "code" && k != "explanation" {
                    let val_str = v
                        .as_str()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| v.to_string());
                    push_line(
                        &mut buf,
                        &format!(
                            "    {} {}: {}",
                            "\u{2022}".bright_black(),
                            k.cyan(),
                            val_str
                        ),
                    );
                }
            }
        } else {
            let explanation = obj.get("explanation").and_then(|v| v.as_str());

            // Print explanation FIRST
            if let Some(exp) = explanation {
                let val_str = exp.to_string();
                push_line(
                    &mut buf,
                    &format!(
                        "    {} {}: {}",
                        "\u{2022}".bright_black(),
                        "explanation".cyan(),
                        val_str
                    ),
                );
            }

            // Define priority categories for parameter ordering
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

            // Collect remaining keys
            let mut remaining_keys: Vec<&String> = Vec::new();
            for k in obj.keys() {
                if k != "explanation" {
                    remaining_keys.push(k);
                }
            }

            // Sort: path-like first, then start, then end, then rest (alphabetically)
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

            // Print parameters in the determined order
            for k in remaining_keys {
                if let Some(v) = obj.get(k) {
                    let val_str = v
                        .as_str()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| v.to_string());
                    push_line(
                        &mut buf,
                        &format!(
                            "    {} {}: {}",
                            "\u{2022}".bright_black(),
                            k.cyan(),
                            val_str
                        ),
                    );
                }
            }
        }
    } else {
        push_line(&mut buf, &format!("    {}", args));
    }

    buf.push_str(&format!("{}\n", "\u{2500}".repeat(width).color(color)));

    crate::cli::pager::page_output(&buf, term_height);
}

pub fn print_tool_result(result: &str) {
    let color = "bright_green";
    let mut out = String::new();

    out.push_str(&format!(
        "  {}\n",
        "\u{2500}\u{2500} Result \u{2500}\u{2500}"
            .color(color)
            .bold()
    ));

    // Try to parse as JSON for pretty printing
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(result) {
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
                push_line(
                    &mut out,
                    &format!("    {} {}", "\u{2022}".bright_black(), message.cyan()),
                );
            }
            push_line(
                &mut out,
                &format!(
                    "    {} {}: {}",
                    "\u{2022}".bright_black(),
                    "path".cyan(),
                    path
                ),
            );

            if !diff.is_empty() {
                push_line(
                    &mut out,
                    &format!("    {} {}:", "\u{2022}".bright_black(), "diff".cyan()),
                );
                for line in diff.lines() {
                    if line.starts_with('+') && !line.starts_with("+++") {
                        push_line(&mut out, &format!("        {}", line.bright_green()));
                    } else if line.starts_with('-') && !line.starts_with("---") {
                        push_line(&mut out, &format!("        {}", line.red()));
                    } else if line.starts_with("@@") {
                        push_line(&mut out, &format!("        {}", line.cyan().dimmed()));
                    } else if line.starts_with("---") || line.starts_with("+++") {
                        push_line(&mut out, &format!("        {}", line.bold().dimmed()));
                    } else {
                        push_line(&mut out, &format!("        {}", line.dimmed()));
                    }
                }
            }
            finish_tool_result(out);
            return;
        }

        // Special handling for command execution results
        if let (Some(_stdout), Some(_stderr), Some(exit_code)) = (
            v.get("stdout").and_then(|v| v.as_str()),
            v.get("stderr").and_then(|v| v.as_str()),
            v.get("exit_code").and_then(|v| v.as_i64()),
        ) {
            let status_color = if exit_code == 0 {
                "bright_green"
            } else {
                "red"
            };
            push_line(
                &mut out,
                &format!(
                    "    {} {}",
                    "Exit Code:".bold(),
                    exit_code.to_string().color(status_color)
                ),
            );
            finish_tool_result(out);
            return;
        }

        // Special handling for "matches" or "results" arrays (e.g., from grep or search)
        for key in ["matches", "results", "files"] {
            if let Some(arr) = v.get(key).and_then(|a| a.as_array()) {
                if arr.is_empty() {
                    push_line(&mut out, &format!("    {}", "(empty results)".dimmed()));
                } else {
                    for item in arr {
                        if let Some(s) = item.as_str() {
                            push_line(
                                &mut out,
                                &format!("    {} {}", "\u{2022}".bright_black(), s.dimmed()),
                            );
                        } else if let Some(obj) = item.as_object() {
                            // Try to format common object structures
                            if let (Some(file), Some(line), Some(text)) = (
                                obj.get("file").and_then(|v| v.as_str()),
                                obj.get("line"),
                                obj.get("text").and_then(|v| v.as_str()),
                            ) {
                                push_line(
                                    &mut out,
                                    &format!(
                                        "    {} {}:{}: {}",
                                        "\u{2022}".bright_black(),
                                        file.cyan(),
                                        line.to_string().yellow(),
                                        text.dimmed()
                                    ),
                                );
                            } else if let (Some(t), Some(path)) = (
                                obj.get("type").and_then(|v| v.as_str()),
                                obj.get("path").and_then(|v| v.as_str()),
                            ) {
                                let mut details = Vec::new();
                                if let Some(size) = obj.get("size").and_then(|v| v.as_u64()) {
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
                                        &format!(
                                            "    {} [{}] {}",
                                            "\u{2022}".bright_black(),
                                            t.cyan(),
                                            path
                                        ),
                                    );
                                } else {
                                    push_line(
                                        &mut out,
                                        &format!(
                                            "    {} [{}] {:<30}  {}",
                                            "\u{2022}".bright_black(),
                                            t.cyan(),
                                            path,
                                            details.join(" | ").dimmed()
                                        ),
                                    );
                                }
                            } else {
                                // Fallback for other objects in the array
                                push_line(
                                    &mut out,
                                    &format!(
                                        "    {} {}",
                                        "\u{2022}".bright_black(),
                                        item.to_string().dimmed()
                                    ),
                                );
                            }
                        }
                    }
                }
                if let Some(truncated) = v.get("truncated").and_then(|v| v.as_bool())
                    && truncated
                {
                    push_line(
                        &mut out,
                        &format!("    {}", "... (results truncated)".yellow().dimmed()),
                    );
                }
                finish_tool_result(out);
                return;
            }
        }

        // Special handling for brave_search results
        if let Some(results) = v.get("results").and_then(|v| v.as_array())
            && v.get("query").is_some()
        {
            for item in results {
                let title = item
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let url = item.get("url").and_then(|v| v.as_str()).unwrap_or_default();
                let snippets = item.get("snippets").and_then(|v| v.as_array());
                push_line(
                    &mut out,
                    &format!(
                        "    {} \x1b]8;;{}\x1b\\{}\x1b]8;;\x1b\\",
                        "\u{2022}".bright_black(),
                        url,
                        title.bold().blue()
                    ),
                );
                if let Some(snip_arr) = snippets {
                    for snippet in snip_arr {
                        if let Some(s) = snippet.as_str() {
                            for line in s.lines() {
                                push_line(&mut out, &format!("      {}", line.dimmed()));
                            }
                        }
                    }
                }
                out.push('\n');
            }
            finish_tool_result(out);
            return;
        }

        if let Ok(pretty) = serde_json::to_string_pretty(&v) {
            // If it's a complex object, show it pretty
            if v.is_object() || v.is_array() {
                for line in pretty.lines() {
                    push_line(&mut out, &format!("    {}", line.dimmed()));
                }
            } else if let Some(s) = v.as_str() {
                // If it's just a string, print it directly (it might have newlines)
                for line in s.lines() {
                    push_line(&mut out, &format!("    {}", line.dimmed()));
                }
            } else {
                push_line(&mut out, &format!("    {}", pretty.dimmed()));
            }
        } else {
            push_line(&mut out, &format!("    {}", result.dimmed()));
        }
    } else {
        // Not JSON, just print it dimmed and indented
        for line in result.lines() {
            push_line(&mut out, &format!("    {}", line.dimmed()));
        }
    }
    out.push('\n');
    finish_tool_result(out);
}

/// Append a line to a string buffer (with trailing newline).
fn push_line(buf: &mut String, line: &str) {
    buf.push_str(line);
    buf.push('\n');
}

/// Print buffered tool result output directly (no pager in React flow).
fn finish_tool_result(out: String) {
    print!("{out}");
}

pub fn print_panel(
    content: &str,
    title: Option<&str>,
    _style: Option<&str>,
    border_style: Option<&str>,
) {
    let term = Term::stdout();
    let (_, term_width) = term.size();
    let width = (term_width as usize).clamp(40, 140);

    let border_color = border_style.unwrap_or("bright_black");

    // Top border
    if let Some(t) = title {
        let title_str = format!(" {} ", t.bold());
        let remaining = width.saturating_sub(title_str.len() + 2);
        println!(
            "{}",
            format!(
                "\u{2500}{}\u{2500}{}",
                title_str,
                "\u{2500}".repeat(remaining)
            )
            .color(border_color)
        );
    } else {
        println!("{}", "\u{2500}".repeat(width).color(border_color));
    }

    // Content with wrapping
    let inner_width = width - 4;
    let options = textwrap::Options::new(inner_width)
        .break_words(false)
        .word_splitter(textwrap::WordSplitter::NoHyphenation);

    for line in content.lines() {
        let wrapped = textwrap::wrap(line, &options);
        for w_line in wrapped {
            println!("    {}", w_line);
        }
    }

    // Bottom border
    println!("{}", "\u{2500}".repeat(width).color(border_color));
}

pub fn report_error(message: &str) {
    eprintln!("{} {}", "NG".red().bold(), message.red());
}

pub fn report_info(message: &str) {
    println!("{} {}", "INFO".cyan().bold(), message.cyan());
}

pub fn report_warning(message: &str) {
    println!("{} {}", "WARNING".yellow().bold(), message.yellow());
}

pub fn report_success(message: &str) {
    println!("{} {}", "OK".bright_green().bold(), message.bright_green());
}

fn format_size_brief(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{}B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1}GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

#[derive(Debug, PartialEq)]
pub enum ConfirmResult {
    Yes,
    No,
    Feedback(String),
}

/// Whether a confirmation prompt accepts free-text feedback or is Yes/No only.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PromptMode {
    /// Input other than Y/N is treated as free-text feedback (for LLM chat sessions).
    WithFeedback,
    /// Only Y/N is accepted; anything else re-prompts.
    YesNoOnly,
}

/// Ask the user a Yes/No confirmation question with configurable prompt mode.
///
/// This function uses a simple stdin/stdout approach instead of dialoguer::Select
/// to avoid terminal raw-mode issues when running under SSH/tmux.
///
/// Supports "y", "yes", "n", "no" (case-insensitive, including full-width),
/// Enter = Yes (default).
/// In `WithFeedback` mode, any other input is treated as feedback.
/// In `YesNoOnly` mode, any other input causes a re-prompt.
fn ask_confirm_with_mode(prompt: &str, mode: PromptMode) -> Option<ConfirmResult> {
    let suffix = match mode {
        PromptMode::WithFeedback => " [Y/n or feedback] ",
        PromptMode::YesNoOnly => " [Y/n] ",
    };
    let y_n = format!("{}{}", prompt, suffix);
    match get_user_input(&y_n) {
        Some(input) => {
            let trimmed = input.trim();
            let lower = trimmed.to_lowercase();
            if lower.is_empty()
                || lower == "y"
                || lower == "yes"
                || lower == "\u{ff59}"
                || lower == "\u{ff59}\u{ff45}\u{ff53}"
            {
                Some(ConfirmResult::Yes)
            } else if lower == "n"
                || lower == "no"
                || lower == "\u{ff4e}"
                || lower == "\u{ff4e}\u{ff4f}"
            {
                Some(ConfirmResult::No)
            } else {
                match mode {
                    PromptMode::WithFeedback => {
                        // Dimmed feedback display
                        println!("  {}", format!("Feedback: {}", trimmed).dimmed());
                        Some(ConfirmResult::Feedback(trimmed.to_string()))
                    }
                    PromptMode::YesNoOnly => {
                        report_warning(&format!(
                            "Unrecognized input '{}'. Please answer Y(es) or N(o).",
                            trimmed
                        ));
                        ask_confirm_with_mode(prompt, mode)
                    }
                }
            }
        }
        None => None, // Interrupted or EOF
    }
}

/// Ask a Yes/No confirmation question that also accepts free-text feedback.
///
/// Use this in LLM chat sessions where the user can provide natural-language
/// feedback as an alternative to a simple Y/N.
pub fn ask_confirm(prompt: &str) -> Option<ConfirmResult> {
    ask_confirm_with_mode(prompt, PromptMode::WithFeedback)
}

/// Ask a Yes/No-only confirmation question (no feedback).
///
/// Use this outside of LLM chat sessions (e.g. startup / initialization)
/// where free-text feedback would be meaningless.
pub fn ask_confirm_simple(prompt: &str) -> Option<ConfirmResult> {
    ask_confirm_with_mode(prompt, PromptMode::YesNoOnly)
}

pub async fn ask_confirm_async(prompt: &str) -> Option<ConfirmResult> {
    let p = prompt.to_string();
    tokio::task::spawn_blocking(move || ask_confirm(&p))
        .await
        .unwrap_or(None)
}

pub async fn ask_confirm_simple_async(prompt: &str) -> Option<ConfirmResult> {
    let p = prompt.to_string();
    tokio::task::spawn_blocking(move || ask_confirm_simple(&p))
        .await
        .unwrap_or(None)
}

pub fn get_user_input(prompt: &str) -> Option<String> {
    // 2. Check for environment override ONLY during tests to prevent accidental production bypass
    #[cfg(test)]
    if std::env::var("LLM_SECURE_TEST_AUTO_APPROVE").is_ok() {
        return Some("y".to_string());
    }

    use rustyline::DefaultEditor;
    use rustyline::error::ReadlineError;

    let mut rl = match DefaultEditor::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create editor: {:?}", e);
            return None;
        }
    };
    match rl.readline(prompt) {
        Ok(line) => Some(line.trim().to_string()),
        Err(ReadlineError::Interrupted) => {
            println!("^C");
            None
        }
        Err(ReadlineError::Eof) => None,
        Err(err) => {
            eprintln!("Error: {:?}", err);
            None
        }
    }
}

pub fn open_external_editor(initial_content: &str) -> anyhow::Result<String> {
    use std::process::Command;
    use tempfile::NamedTempFile;

    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| {
            if cfg!(windows) {
                "notepad".to_string()
            } else {
                "vi".to_string()
            }
        });

    let mut file = NamedTempFile::new()?;
    if !initial_content.is_empty() {
        file.write_all(initial_content.as_bytes())?;
    }

    let status = Command::new(editor).arg(file.path()).status()?;

    if !status.success() {
        return Err(anyhow::anyhow!("Editor exited with error status"));
    }

    let mut content = String::new();
    let mut file = std::fs::File::open(file.path())?;
    file.read_to_string(&mut content)?;

    Ok(content)
}
