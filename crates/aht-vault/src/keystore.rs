//! Storage for the Secret Key on a device, so users only type it once per device.

use crate::error::{Result, VaultError};
use crate::fsutil::atomic_write;
use crate::secret_key::SecretKey;
use std::path::PathBuf;

pub trait SecretKeyStore: Send + Sync {
    fn get(&self, vault_id: &str) -> Result<Option<SecretKey>>;
    fn set(&self, vault_id: &str, key: &SecretKey) -> Result<()>;
    /// Deleting a missing entry is not an error.
    fn delete(&self, vault_id: &str) -> Result<()>;
}

fn check_vault_id(vault_id: &str) -> Result<()> {
    if vault_id.is_empty()
        || !vault_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(VaultError::Invalid(format!("bad vault id {vault_id:?}")));
    }
    Ok(())
}

/// Plain-file store (`<dir>/<vault_id>.secretkey`). For tests and as a fallback where no OS
/// keychain exists; the file is only protected by filesystem permissions.
#[derive(Clone, Debug)]
pub struct FileSecretKeyStore {
    dir: PathBuf,
}

impl FileSecretKeyStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path(&self, vault_id: &str) -> Result<PathBuf> {
        check_vault_id(vault_id)?;
        Ok(self.dir.join(format!("{vault_id}.secretkey")))
    }
}

impl SecretKeyStore for FileSecretKeyStore {
    fn get(&self, vault_id: &str) -> Result<Option<SecretKey>> {
        match std::fs::read_to_string(self.path(vault_id)?) {
            Ok(s) => {
                let s = zeroize::Zeroizing::new(s);
                SecretKey::parse(&s).map(Some)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn set(&self, vault_id: &str, key: &SecretKey) -> Result<()> {
        let path = self.path(vault_id)?;
        std::fs::create_dir_all(&self.dir)?;
        atomic_write(&path, key.reveal().as_bytes())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    fn delete(&self, vault_id: &str) -> Result<()> {
        match std::fs::remove_file(self.path(vault_id)?) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }
}

/// OS keychain store (Windows Credential Manager, macOS Keychain, Secret Service).
#[cfg(feature = "os-keychain")]
#[derive(Clone, Debug)]
pub struct OsKeychainStore {
    service: String,
}

#[cfg(feature = "os-keychain")]
impl OsKeychainStore {
    /// `service` namespaces entries, e.g. `"Handrail"`.
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, vault_id: &str) -> Result<keyring::Entry> {
        check_vault_id(vault_id)?;
        keyring::Entry::new(&self.service, &format!("vault-secret-key:{vault_id}"))
            .map_err(|e| VaultError::Keychain(e.to_string()))
    }
}

#[cfg(feature = "os-keychain")]
impl SecretKeyStore for OsKeychainStore {
    fn get(&self, vault_id: &str) -> Result<Option<SecretKey>> {
        match self.entry(vault_id)?.get_password() {
            Ok(s) => {
                let s = zeroize::Zeroizing::new(s);
                SecretKey::parse(&s).map(Some)
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(VaultError::Keychain(e.to_string())),
        }
    }

    fn set(&self, vault_id: &str, key: &SecretKey) -> Result<()> {
        self.entry(vault_id)?
            .set_password(&key.reveal())
            .map_err(|e| VaultError::Keychain(e.to_string()))
    }

    fn delete(&self, vault_id: &str) -> Result<()> {
        match self.entry(vault_id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(VaultError::Keychain(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exercise(store: &dyn SecretKeyStore, vault_id: &str) {
        assert!(store.get(vault_id).unwrap().is_none());
        let k = SecretKey::generate().unwrap();
        store.set(vault_id, &k).unwrap();
        assert_eq!(store.get(vault_id).unwrap(), Some(k.clone()));
        let k2 = SecretKey::generate().unwrap();
        store.set(vault_id, &k2).unwrap();
        assert_eq!(store.get(vault_id).unwrap(), Some(k2));
        store.delete(vault_id).unwrap();
        assert!(store.get(vault_id).unwrap().is_none());
        store.delete(vault_id).unwrap();
    }

    #[test]
    fn file_store_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSecretKeyStore::new(dir.path().join("keys"));
        exercise(&store, "abc123");
        assert!(store.get("../evil").is_err());
    }

    #[cfg(feature = "os-keychain")]
    #[test]
    #[ignore = "touches the real OS keychain"]
    fn os_keychain_round_trip() {
        let store = OsKeychainStore::new("Handrail-test");
        let id = crate::crypto::random_hex(8).unwrap();
        exercise(&store, &id);
    }
}
