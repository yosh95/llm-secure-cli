pub mod chat_logger;
pub mod http;
pub mod logging;
pub mod media;
pub mod session_store;

pub fn hex_encode(data: impl AsRef<[u8]>) -> String {
    data.as_ref().iter().map(|b| format!("{:02x}", b)).collect()
}

pub fn format_number<T: std::fmt::Display>(n: T) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut result = String::new();

    for (i, &b) in bytes.iter().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(b as char);
    }
    result.chars().rev().collect()
}
