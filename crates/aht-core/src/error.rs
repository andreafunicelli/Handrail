use serde::{Deserialize, Serialize};

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Cross-crate error. Crates keep their own detailed errors internally and convert at the boundary.
#[derive(Debug, Clone, thiserror::Error, Serialize, Deserialize)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
pub enum Error {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("vault is locked")]
    VaultLocked,
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("host key mismatch for {0}")]
    HostKeyMismatch(String),
    #[error("permission denied: {0}")]
    Denied(String),
    #[error("session busy: {0}")]
    Busy(String),
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("i/o error: {0}")]
    Io(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("provider error: {0}")]
    Provider(String),
    #[error("{0}")]
    Other(String),
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e.to_string())
    }
}
