use colored::Colorize;

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
