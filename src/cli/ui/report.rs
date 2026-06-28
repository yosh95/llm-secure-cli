pub fn report_error(message: &str) {
    eprintln!("❌ {}", message);
}

pub fn report_info(message: &str) {
    eprintln!("🔵 {}", message);
}

/// Report an LLM/verifier query in progress.
pub fn report_querying(message: &str) {
    eprintln!("\u{1f3c3} {}", message);
}

pub fn report_warning(message: &str) {
    eprintln!("🟡 {}", message);
}

pub fn report_success(message: &str) {
    eprintln!("✅ {}", message);
}
