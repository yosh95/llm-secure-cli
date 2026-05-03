pub mod chat_logger;
pub mod http;
pub mod logging;
pub mod media;

pub fn hex_encode(data: impl AsRef<[u8]>) -> String {
    data.as_ref().iter().map(|b| format!("{:02x}", b)).collect()
}
