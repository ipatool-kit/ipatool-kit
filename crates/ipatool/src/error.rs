use thiserror::Error;

pub type Result<T> = std::result::Result<T, IpatoolError>;

#[derive(Debug, Error)]
pub enum IpatoolError {
    #[error("{0}")]
    Message(String),

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
