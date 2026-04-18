use colored::*;
use console::Term;
use termimad::MadSkin;
use textwrap::wrap;

pub fn print_block(content: &str, title: Option<&str>, style: Option<&str>) {
    let term = Term::stdout();
    let (_, width) = term.size();
    let width = (width as usize).min(100);

    if let Some(t) = title {
        let rule_color = style.unwrap_or("cyan");
        let rule = "-".repeat(width);
        println!("{}", rule.color(rule_color));
        println!("{}", t.bold().color(rule_color));
    }

    // Use termimad for markdown rendering
    let mut skin = MadSkin::default();
    skin.set_headers_fg(termimad::crossterm::style::Color::Cyan);
    skin.print_text(content);

    if title.is_some() {
        let rule_color = style.unwrap_or("cyan");
        let rule = "-".repeat(width);
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
    println!();
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
            if let Some(exp) = explanation {
                println!(
                    "    {} {}: {}",
                    "•".bright_black(),
                    "explanation".cyan(),
                    exp
                );
            }
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
                        println!("        {}", line.green());
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

            // Print other arguments if any
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
        } else {
            for (k, v) in obj {
                let val_str = if v.is_string() {
                    v.as_str().unwrap().to_string()
                } else {
                    v.to_string()
                };
                // Indent and use a softer bullet
                println!("    {} {}: {}", "•".bright_black(), k.cyan(), val_str);
            }
        }
    } else {
        println!("    {}", args);
    }
}

pub fn print_tool_result(result: &str) {
    let color = "green";
    println!("  {}", "── Result ──".color(color).dimmed());

    // Try to parse as JSON for pretty printing
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(result) {
        // Special handling for command execution results
        if let (Some(stdout), Some(stderr), Some(exit_code)) = (
            v.get("stdout").and_then(|v| v.as_str()),
            v.get("stderr").and_then(|v| v.as_str()),
            v.get("exit_code").and_then(|v| v.as_i64()),
        ) {
            let status_color = if exit_code == 0 { "green" } else { "red" };
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
                    println!("      {}", line.red().dimmed());
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
            format!("-{}-{}-", title_str, "-".repeat(remaining)).color(border_color)
        );
    } else {
        println!(
            "{}",
            format!("-{}-", "-".repeat(width - 2)).color(border_color)
        );
    }

    // Content with wrapping
    let inner_width = width - 4;
    for line in content.lines() {
        let wrapped = wrap(line, inner_width);
        for w_line in wrapped {
            println!(
                "{} {:inner_width$} {}",
                "│".color(border_color),
                w_line,
                "│".color(border_color),
                inner_width = inner_width
            );
        }
    }

    // Bottom border
    println!(
        "{}",
        format!("-{}-", "-".repeat(width - 2)).color(border_color)
    );
}

pub fn report_error(message: &str) {
    eprintln!("{} {}", "NG".red().bold(), message.red());
}

pub fn report_warning(message: &str) {
    println!("{} {}", "WARNING".yellow().bold(), message.yellow());
}

pub fn report_success(message: &str) {
    println!("{} {}", "OK".green().bold(), message.green());
}

pub fn get_user_input(prompt: &str) -> String {
    use std::io::{self, Write};
    print!("{}", prompt);
    let _ = io::stdout().flush();
    let mut input = String::new();
    let _ = io::stdin().read_line(&mut input);
    input.trim().to_string()
}

pub fn open_external_editor(initial_content: &str) -> anyhow::Result<String> {
    use std::io::{Read, Write};
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
