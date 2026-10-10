//! Application state shared by every IPC command.

use aht_core::{Error, Result};
use aht_vault::{SecretKeyStore, Vault};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct Paths {
    pub data_dir: PathBuf,
    pub vault_dir: PathBuf,
    pub profiles_dir: PathBuf,
    pub sync_state_file: PathBuf,
}

impl Paths {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            vault_dir: data_dir.join("vault"),
            profiles_dir: data_dir.join("harness"),
            sync_state_file: data_dir.join("sync-state.json"),
            data_dir,
        }
    }
}

pub struct AppState {
    pub paths: Paths,
    pub vault: Arc<Mutex<Vault>>,
    pub keystore: Arc<dyn SecretKeyStore>,
    /// Import previews kept server-side so secrets never round-trip through the UI.
    pub import_previews: Mutex<HashMap<String, aht_import::ImportBundle>>,
    pub github_device: Mutex<Option<aht_sync::github_device_flow::DeviceCode>>,
    pub pending_github_token: Mutex<Option<String>>,
    /// Held for a whole sync round and while sync settings change: one sync at a time.
    pub sync_lock: Mutex<()>,
    pub terminal: crate::terminal::TerminalHub,
}

impl AppState {
    pub fn new(data_dir: PathBuf) -> Result<Self> {
        let paths = Paths::new(data_dir);
        std::fs::create_dir_all(&paths.vault_dir)?;
        let vault = Vault::open(&paths.vault_dir).map_err(Error::from)?;
        Ok(Self {
            keystore: default_keystore(&paths),
            vault: Arc::new(Mutex::new(vault)),
            import_previews: Mutex::new(HashMap::new()),
            github_device: Mutex::new(None),
            pending_github_token: Mutex::new(None),
            sync_lock: Mutex::new(()),
            terminal: crate::terminal::TerminalHub::default(),
            paths,
        })
    }
}

fn default_keystore(paths: &Paths) -> Arc<dyn SecretKeyStore> {
    // The OS keychain holds the Secret Key so returning users only type their master password.
    // Linux desktops without a secret service fall back to a file in the app data dir.
    if std::env::var_os("AIHUBTERM_FILE_KEYSTORE").is_some() {
        return Arc::new(aht_vault::FileSecretKeyStore::new(
            paths.data_dir.join("keys"),
        ));
    }
    Arc::new(aht_vault::OsKeychainStore::new("dev.aihubterm.app"))
}

pub fn err(e: impl std::fmt::Display) -> Error {
    Error::Other(e.to_string())
}
