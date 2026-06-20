pub fn report_error(message: &str) {
    eprintln!("NG {}", message);
}

pub fn report_info(message: &str) {
    eprintln!("INFO {}", message);
}

pub fn report_warning(message: &str) {
    eprintln!("WARNING {}", message);
}

pub fn report_success(message: &str) {
    eprintln!("OK {}", message);
}
