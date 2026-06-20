pub mod chat_logger;
pub mod http;
pub mod logging;
pub mod media;
pub mod python_highlighter;
pub mod session_store;

const HEX_CHARS: [char; 16] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f',
];

pub fn hex_encode(data: impl AsRef<[u8]>) -> String {
    let bytes = data.as_ref();
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX_CHARS[((b >> 4) & 0x0f) as usize]);
        s.push(HEX_CHARS[(b & 0x0f) as usize]);
    }
    s
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
