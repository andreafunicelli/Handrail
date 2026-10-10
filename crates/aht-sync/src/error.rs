pub type Result<T, E = SyncError> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    /// The etag the caller expected no longer matches the remote (concurrent writer).
    #[error("conflict: remote changed concurrently ({0})")]
    Conflict(String),
    #[error("remote file not found: {0}")]
    NotFound(String),
    #[error("rate limited; retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },
    /// The remote manifest differs from the local one (re-keying is not supported in v1).
    #[error("remote manifest differs from the local vault manifest")]
    ManifestMismatch,
    #[error("remote has no vault manifest")]
    NoRemoteVault,
    #[error("local vault is not initialized; join it with the remote manifest first")]
    LocalNotInitialized,
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("invalid path: {0}")]
    InvalidPath(String),
    #[error("http error: {0}")]
    Http(String),
    #[error("unexpected response: {0}")]
    Protocol(String),
    #[error("vault error: {0}")]
    Vault(#[from] aht_vault::VaultError),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("gave up after repeated concurrent changes on the remote")]
    TooManyRetries,
}

impl From<reqwest::Error> for SyncError {
    fn from(e: reqwest::Error) -> Self {
        SyncError::Http(e.without_url().to_string())
    }
}

impl From<SyncError> for aht_core::Error {
    fn from(e: SyncError) -> Self {
        use aht_core::Error as E;
        match e {
            SyncError::Auth(s) => E::Auth(s),
            SyncError::NotFound(s) => E::NotFound(s),
            SyncError::Http(_) | SyncError::RateLimited { .. } => E::Network(e.to_string()),
            SyncError::Io(io) => E::Io(io.to_string()),
            SyncError::Vault(aht_vault::VaultError::Locked) => E::VaultLocked,
            other => E::Other(other.to_string()),
        }
    }
}
