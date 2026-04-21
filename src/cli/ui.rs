use crate::cli::markdown::render_markdown;
use colored::*;
use console::Term;
use std::io::{self, Read, Write};
use textwrap::wrap;

pub fn print_block(content: &str, title: Option<&str>, style: Option<&str>) {
    let term = Term::stdout();
    let (_, width) = term.size();
    let width = (width as usize).min(100);

    if let Some(t) = title {
        let rule_color = style.unwrap_or("cyan");
        let rule = "─".repeat(width);
        println!("{}", rule.color(rule_color));
        println!("{}", t.bold().color(rule_color));
    }

    // Use our custom markdown renderer
    let output = render_markdown(content.trim(), width);
    println!("{}", output);

    if title.is_some() {
        let rule_color = style.unwrap_or("cyan");
        let rule = "─".repeat(width);
        println!("{}", rule.color(rule_color));
    }
}

pub fn print_rule(title: Option<&str>, style: Option<&str>) {
    let term = Term::stdout();
    let (_, width) = term.size();
    let width = (width as usize).min(100);
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
            "─".repeat(left).color(color),
            title_display.color(color),
            "─".repeat(right).color(color)
        );
    } else {
        println!("{}", "─".repeat(width).color(color));
    }
}

pub fn print_key_value(key: &str, value: &str) {
    println!("  {:15} {}", key.bold().cyan(), value);
}

pub fn print_tool_call(name: &str, args: &serde_json::Value) {
    let term = Term::stdout();
    let (_, width) = term.size();
    let width = (width as usize).min(100);
    let color = "yellow";

    println!();
    println!("{}", "─".repeat(width).color(color));
    println!(
        "{} {}{}",
        "->".yellow().bold(),
        name.bold().yellow(),
        ":".yellow()
    );
    if let Some(obj) = args.as_object() {
        if name == "edit_file" {
            let path = obj.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let search = obj.get("search").and_then(|v| v.as_str()).unwrap_or("");
            let replace = obj.get("replace").and_then(|v| v.as_str()).unwrap_or("");
            let explanation = obj.get("explanation").and_then(|v| v.as_str());

            println!("    {} {}: {}", "•".bright_black(), "path".cyan(), path);
            println!("    {} {}:", "•".bright_black(), "diff".cyan());

            let diff = difflib::unified_diff(
                &search.lines().collect::<Vec<_>>(),
                &replace.lines().collect::<Vec<_>>(),
                "search",
                "replace",
                "",
                "",
                3,
            );

            if diff.is_empty() && !search.is_empty() && search == replace {
                println!("        {}", "(no changes)".dimmed());
            } else {
                for line in diff {
                    if line.starts_with('+') && !line.starts_with("+++") {
                        println!("        {}", line.cyan());
                    } else if line.starts_with('-') && !line.starts_with("---") {
                        println!("        {}", line.red());
                    } else if !line.starts_with("@@")
                        && !line.starts_with("---")
                        && !line.starts_with("+++")
                    {
                        println!("        {}", line.dimmed());
                    }
                }
            }

            // Print other arguments if any (except explanation which we want last)
            for (k, v) in obj {
                if k != "path" && k != "search" && k != "replace" && k != "explanation" {
                    let val_str = if v.is_string() {
                        v.as_str().unwrap().to_string()
                    } else {
                        v.to_string()
                    };
                    println!("    {} {}: {}", "•".bright_black(), k.cyan(), val_str);
                }
            }

            // Print explanation last for edit_file
            if let Some(exp) = explanation {
                println!(
                    "    {} {}: {}",
                    "•".bright_black(),
                    "explanation".cyan(),
                    exp
                );
            }
        } else {
            // Print all arguments except explanation
            for (k, v) in obj {
                if k != "explanation" {
                    let val_str = if v.is_string() {
                        v.as_str().unwrap().to_string()
                    } else {
                        v.to_string()
                    };
                    println!("    {} {}: {}", "•".bright_black(), k.cyan(), val_str);
                }
            }
            // Print explanation last
            if let Some(v) = obj.get("explanation") {
                let val_str = if v.is_string() {
                    v.as_str().unwrap().to_string()
                } else {
                    v.to_string()
                };
                println!(
                    "    {} {}: {}",
                    "•".bright_black(),
                    "explanation".cyan(),
                    val_str
                );
            }
        }
    } else {
        println!("    {}", args);
    }
    println!("{}", "─".repeat(width).color(color));
}

pub fn print_tool_result(result: &str) {
    let color = "bright_green";
    println!("  {}", "── Result ──".color(color).bold());

    // Try to parse as JSON for pretty printing
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(result) {
        // Special handling for command execution results
        if let (Some(stdout), Some(stderr), Some(exit_code)) = (
            v.get("stdout").and_then(|v| v.as_str()),
            v.get("stderr").and_then(|v| v.as_str()),
            v.get("exit_code").and_then(|v| v.as_i64()),
        ) {
            let status_color = if exit_code == 0 {
                "bright_green"
            } else {
                "red"
            };
            println!(
                "    {} {}",
                "Exit Code:".bold(),
                exit_code.to_string().color(status_color)
            );

            if !stdout.is_empty() {
                println!("    {}", "STDOUT:".bold().cyan());
                for line in stdout.lines() {
                    println!("      {}", line.dimmed());
                }
            }

            if !stderr.is_empty() {
                println!("    {}", "STDERR:".bold().red());
                for line in stderr.lines() {
                    println!("      {}", line.red());
                }
            }
            return;
        }

        // Special handling for "matches" or "results" arrays (e.g., from grep or search)
        for key in ["matches", "results", "files"] {
            if let Some(arr) = v.get(key).and_then(|a| a.as_array()) {
                if arr.is_empty() {
                    println!("    {}", "(empty results)".dimmed());
                } else {
                    for item in arr {
                        if let Some(s) = item.as_str() {
                            println!("    {} {}", "•".bright_black(), s.dimmed());
                        } else if let Some(obj) = item.as_object() {
                            // Try to format common object structures
                            if let (Some(file), Some(line), Some(text)) = (
                                obj.get("file").and_then(|v| v.as_str()),
                                obj.get("line"),
                                obj.get("text").and_then(|v| v.as_str()),
                            ) {
                                println!(
                                    "    {} {}:{}: {}",
                                    "•".bright_black(),
                                    file.cyan(),
                                    line.to_string().yellow(),
                                    text.dimmed()
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
                                    println!("    {} [{}] {}", "•".bright_black(), t.cyan(), path);
                                } else {
                                    println!(
                                        "    {} [{}] {:<30}  {}",
                                        "•".bright_black(),
                                        t.cyan(),
                                        path,
                                        details.join(" | ").dimmed()
                                    );
                                }
                            } else {
                                // Fallback for other objects in the array
                                println!(
                                    "    {} {}",
                                    "•".bright_black(),
                                    item.to_string().dimmed()
                                );
                            }
                        }
                    }
                }
                if let Some(truncated) = v.get("truncated").and_then(|v| v.as_bool()) {
                    if truncated {
                        println!("    {}", "... (results truncated)".yellow().dimmed());
                    }
                }
                return;
            }
        }

        // Special handling for brave_search results
        if let Some(results) = v.get("results").and_then(|v| v.as_array()) {
            if v.get("query").is_some() {
                for item in results {
                    let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("");
                    let url = item.get("url").and_then(|v| v.as_str()).unwrap_or("");
                    let snippet = item
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    println!("    {} {}", "•".bright_black(), title.bold().cyan());
                    println!("      {}", url.dimmed().underline());
                    if !snippet.is_empty() {
                        for line in snippet.lines() {
                            println!("      {}", line.dimmed());
                        }
                    }
                    println!();
                }
                return;
            }
        }

        // Special handling for read_url_content or similar "content" responses
        if let Some(content) = v.get("content").and_then(|v| v.as_str()) {
            for line in content.lines().take(20) {
                println!("    {}", line.dimmed());
            }
            if content.lines().count() > 20 {
                println!("    {}", "... (long content truncated in display)".dimmed());
            }
            if let Some(notes) = v.get("notes").and_then(|v| v.as_array()) {
                for note in notes {
                    if let Some(n) = note.as_str() {
                        println!("    {} {}", "!".yellow(), n.yellow().dimmed());
                    }
                }
            }
            return;
        }

        if let Ok(pretty) = serde_json::to_string_pretty(&v) {
            // If it's a complex object, show it pretty
            if v.is_object() || v.is_array() {
                for line in pretty.lines() {
                    println!("    {}", line.dimmed());
                }
            } else if v.is_string() {
                // If it's just a string, print it directly (it might have newlines)
                for line in v.as_str().unwrap().lines() {
                    println!("    {}", line.dimmed());
                }
            } else {
                println!("    {}", pretty.dimmed());
            }
        } else {
            println!("    {}", result.dimmed());
        }
    } else {
        // Not JSON, just print it dimmed and indented
        for line in result.lines() {
            println!("    {}", line.dimmed());
        }
    }
    println!();
}

pub fn print_panel(
    content: &str,
    title: Option<&str>,
    _style: Option<&str>,
    border_style: Option<&str>,
) {
    let term = Term::stdout();
    let (_, term_width) = term.size();
    let width = (term_width as usize).clamp(40, 100);

    let border_color = border_style.unwrap_or("bright_black");

    // Top border
    if let Some(t) = title {
        let title_str = format!(" {} ", t.bold());
        let remaining = width.saturating_sub(title_str.len() + 2);
        println!(
            "{}",
            format!("─{}─{}", title_str, "─".repeat(remaining)).color(border_color)
        );
    } else {
        println!("{}", "─".repeat(width).color(border_color));
    }

    // Content with wrapping
    let inner_width = width - 4;
    for line in content.lines() {
        let wrapped = wrap(line, inner_width);
        for w_line in wrapped {
            println!("    {}", w_line);
        }
    }

    // Bottom border
    println!("{}", "─".repeat(width).color(border_color));
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

pub fn ask_confirm(prompt: &str) -> bool {
    let term = Term::stdout();
    print!("{} (y/N): ", prompt);
    let _ = io::stdout().flush();

    loop {
        if let Ok(key) = term.read_char() {
            match key {
                'y' | 'Y' | 'ｙ' | 'Ｙ' => {
                    println!("{}", "yes".bright_green());
                    return true;
                }
                'n' | 'N' | 'ｎ' | 'Ｎ' | '\r' | '\n' => {
                    println!("{}", "no".red());
                    return false;
                }
                '\u{3}' => {
                    // Ctrl+C
                    println!("^C");
                    std::process::exit(0);
                }
                _ => {}
            }
        }
    }
}

pub fn get_user_input(prompt: &str) -> String {
    use rustyline::error::ReadlineError;
    use rustyline::DefaultEditor;

    let mut rl = DefaultEditor::new().expect("Failed to create editor");
    match rl.readline(prompt) {
        Ok(line) => line.trim().to_string(),
        Err(ReadlineError::Interrupted) => {
            println!("^C");
            std::process::exit(0);
        }
        Err(ReadlineError::Eof) => {
            std::process::exit(0);
        }
        Err(err) => {
            eprintln!("Error: {:?}", err);
            std::process::exit(1);
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
