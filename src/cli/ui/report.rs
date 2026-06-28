pub fn report_error(message: &str) {
    eprintln!("❌ {}", message);
}

pub fn report_info(message: &str) {
    eprintln!("🔵 {}", message);
}

pub fn report_warning(message: &str) {
    eprintln!("🟡 {}", message);
}

pub fn report_success(message: &str) {
    eprintln!("✅ {}", message);
}
