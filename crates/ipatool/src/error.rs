use thiserror::Error;

pub type Result<T> = std::result::Result<T, IpatoolError>;

#[derive(Debug, Error)]
pub enum IpatoolError {
    #[error("{0}")]
    Message(String),

    #[error("authentication tag mismatch")]
    AuthTagMismatch,

    #[error("invalid MAC address: {0}")]
    InvalidMac(String),

    #[error("country code mapping for store front ({0}) was not found")]
    UnknownStorefront(String),

    #[error("hardware ID must be 1-20 bytes")]
    InvalidHardwareId,

    #[error("{op} is not implemented yet (requires native App Store SAP signing)")]
    NotImplemented { op: &'static str },

    #[error("HTTP error: {0}")]
    Http(String),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("plist error: {0}")]
    Plist(String),
}

impl IpatoolError {
    pub fn msg(s: impl Into<String>) -> Self {
        Self::Message(s.into())
    }
}
