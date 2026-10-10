//! Vault-local error type, converted to `aht_core::Error` at the crate boundary.

pub type Result<T, E = VaultError> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    /// Master password or Secret Key is wrong (key check failed).
    #[error("wrong master password or secret key")]
    WrongCredentials,
    #[error("vault is locked")]
    Locked,
    #[error("vault is not initialized")]
    NotInitialized,
    #[error("vault already exists at this location")]
    AlreadyInitialized,
    /// Ciphertext failed authentication or a file is malformed.
    #[error("corrupt or tampered data: {0}")]
    Corrupt(String),
    #[error("invalid secret key: {0}")]
    InvalidSecretKey(String),
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("unsupported vault format version {0}")]
    UnsupportedVersion(u32),
    #[error("key derivation failed: {0}")]
    Kdf(String),
    #[error("keychain error: {0}")]
    Keychain(String),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

impl From<VaultError> for aht_core::Error {
    fn from(e: VaultError) -> Self {
        use aht_core::Error as E;
        match e {
            VaultError::WrongCredentials => E::Auth(e.to_string()),
            VaultError::Locked => E::VaultLocked,
            VaultError::NotFound(s) => E::NotFound(s),
            VaultError::Invalid(_) | VaultError::InvalidSecretKey(_) => E::Invalid(e.to_string()),
            VaultError::Io(io) => E::Io(io.to_string()),
            other => E::Other(other.to_string()),
        }
    }
}
