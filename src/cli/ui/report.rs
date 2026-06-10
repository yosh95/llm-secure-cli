use colored::Colorize;

pub fn report_error(message: &str) {
    eprintln!("{} {}", "NG".red().bold(), message.red());
}

pub fn report_info(message: &str) {
    eprintln!("{} {}", "INFO".cyan().bold(), message.cyan());
}

pub fn report_warning(message: &str) {
    eprintln!("{} {}", "WARNING".yellow().bold(), message.yellow());
}

pub fn report_success(message: &str) {
    eprintln!("{} {}", "OK".bright_green().bold(), message.bright_green());
}
