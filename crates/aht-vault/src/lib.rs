//! aht-vault — encrypted, sync-friendly item store (spec §5).
//!
//! Argon2id(master password) + 128-bit Secret Key → HKDF-SHA256 → vault key; every item is a
//! separate XChaCha20-Poly1305 file with an opaque name, merged across devices by HLC
//! last-writer-wins. See [`Vault`].

mod crypto;
mod error;
mod fsutil;
mod hlc;
mod item;
mod keystore;
mod secret_key;
mod vault;

pub use crypto::KdfParams;
pub use error::{Result, VaultError};
pub use hlc::{Hlc, HlcClock};
pub use item::{Setting, VaultItem};
#[cfg(feature = "os-keychain")]
pub use keystore::OsKeychainStore;
pub use keystore::{FileSecretKeyStore, SecretKeyStore};
pub use secret_key::SecretKey;
pub use vault::{
    ConflictRecord, Envelope, Manifest, MergeOutcome, RawFile, SnapshotInfo, Vault, VaultStatus,
    FORMAT_VERSION, MAX_SNAPSHOTS,
};
