use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("file truncated while reading {0}")]
    Truncated(&'static str),
    #[error("invalid metadata JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("preview is not a PNG")]
    NotPng,
    #[error("bad record at offset {offset}: {reason}")]
    BadRecord { offset: usize, reason: &'static str },
    #[error("record at offset {offset}: CRC mismatch (stored {stored:#04x}, computed {computed:#04x})")]
    BadCrc { offset: usize, stored: u8, computed: u8 },
    #[error("invalid NRBF stream: {0}")]
    Nrbf(String),
    #[error("invalid protobuf: {0}")]
    Protobuf(String),
    #[error("record payload too large ({0} bytes, max 253)")]
    PayloadTooLarge(usize),
}

pub type Result<T> = std::result::Result<T, Error>;
