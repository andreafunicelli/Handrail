//! The encrypted item store.
//!
//! Layout under the vault directory:
//! - `manifest.json` — plaintext: format version, vault id, KDF params, salt, AEAD key check.
//! - `items/<file_id>.enc` — one encrypted [`Envelope`] per logical item (incl. tombstones).
//! - `snapshots/<ms>-<rand>.snap` — encrypted bundles of all item files (last 10 kept).
//! - `local/` — device-local state, never synced: device id, conflict history.
//!
//! `manifest.json` and `items/` are what sync uploads ([`Vault::raw_files`]).

use crate::crypto::{random_hex, KdfParams, VaultKey};
use crate::error::{Result, VaultError};
use crate::fsutil::atomic_write;
use crate::hlc::{wall_now_ms, Hlc, HlcClock};
use crate::item::VaultItem;
use crate::secret_key::SecretKey;
use aht_core::DeviceId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;
use zeroize::Zeroizing;

pub const FORMAT_VERSION: u32 = 1;
const MANIFEST: &str = "manifest.json";
const ITEMS: &str = "items";
const SNAPSHOTS: &str = "snapshots";
const LOCAL: &str = "local";
const HISTORY: &str = "history";
const KEY_CHECK_PLAINTEXT: &[u8] = b"aihubterm vault key check v1";
const SNAPSHOT_MAGIC: &[u8] = b"AHTSNAP1";
/// Number of snapshots kept on disk.
pub const MAX_SNAPSHOTS: usize = 10;

/// Plaintext vault manifest, readable before unlock.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format_version: u32,
    pub vault_id: String,
    pub kdf_algorithm: String,
    pub kdf: KdfParams,
    /// Hex, 16 bytes.
    pub salt: String,
    /// Hex `nonce ‖ ct` of a known plaintext; decrypts only with the right vault key.
    pub key_check: String,
}

impl Manifest {
    fn key_check_aad(&self) -> Vec<u8> {
        format!(
            "aht-manifest:v{}:{}:{}",
            self.format_version, self.vault_id, self.salt
        )
        .into_bytes()
    }

    fn parse(bytes: &[u8]) -> Result<Self> {
        let m: Manifest = serde_json::from_slice(bytes)
            .map_err(|e| VaultError::Corrupt(format!("manifest: {e}")))?;
        if m.format_version != FORMAT_VERSION {
            return Err(VaultError::UnsupportedVersion(m.format_version));
        }
        if m.kdf_algorithm != "argon2id" {
            return Err(VaultError::Corrupt(format!(
                "unknown kdf {}",
                m.kdf_algorithm
            )));
        }
        Ok(m)
    }
}

/// Decrypted content of an item file.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub id: String,
    pub item_type: String,
    pub updated_at: Hlc,
    pub device_id: DeviceId,
    pub deleted: bool,
    pub payload: Value,
}

impl fmt::Debug for Envelope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Envelope")
            .field("id", &self.id)
            .field("item_type", &self.item_type)
            .field("updated_at", &self.updated_at)
            .field("device_id", &self.device_id)
            .field("deleted", &self.deleted)
            .field("payload", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultStatus {
    pub initialized: bool,
    pub locked: bool,
}

/// A syncable file: relative path with `/` separators, raw bytes, SHA-256 hex of the bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct RawFile {
    pub path: String,
    pub bytes: Vec<u8>,
    pub sha256: String,
}

impl fmt::Debug for RawFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RawFile")
            .field("path", &self.path)
            .field("len", &self.bytes.len())
            .field("sha256", &self.sha256)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeOutcome {
    /// File did not exist locally and was added.
    Added,
    /// Remote version was newer and replaced the local one (local kept in history).
    RemoteWon,
    /// Local version is newer; remote kept in history, nothing written.
    LocalWon,
    /// Identical version already present.
    Unchanged,
}

/// A version that lost a last-writer-wins merge.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConflictRecord {
    pub recorded_at_ms: u64,
    pub file_id: String,
    pub winner: Hlc,
    pub loser: Envelope,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SnapshotInfo {
    /// File name inside `snapshots/`; pass to [`Vault::restore_snapshot`].
    pub name: String,
    pub created_ms: u64,
    pub size: u64,
}

struct Entry {
    item_type: String,
    id: String,
    updated_at: Hlc,
    deleted: bool,
    /// Serialized envelope JSON.
    plaintext: Zeroizing<Vec<u8>>,
}

impl Entry {
    fn envelope(&self) -> Result<Envelope> {
        Ok(serde_json::from_slice(&self.plaintext)?)
    }
}

struct Unlocked {
    key: VaultKey,
    /// file_id → decrypted entry.
    index: HashMap<String, Entry>,
    corrupt: Vec<String>,
}

pub struct Vault {
    dir: PathBuf,
    manifest: Option<Manifest>,
    clock: HlcClock,
    unlocked: Option<Unlocked>,
    /// Bumped on every change to the item files; see [`Vault::revision`].
    revision: u64,
}

impl fmt::Debug for Vault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Vault")
            .field("dir", &self.dir)
            .field("status", &self.status())
            .field("items", &self.unlocked.as_ref().map(|u| u.index.len()))
            .finish_non_exhaustive()
    }
}

fn item_aad(file_id: &str) -> Vec<u8> {
    format!("aht-item:v{FORMAT_VERSION}:{file_id}").into_bytes()
}

fn is_file_id(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn read_opt(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn sorted_dir(dir: &Path, ext: &str) -> Result<Vec<(String, PathBuf)>> {
    let mut out = Vec::new();
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e.into()),
    };
    for entry in rd {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(stem) = name.strip_suffix(ext) {
            if !stem.starts_with('.') && entry.file_type()?.is_file() {
                out.push((stem.to_string(), entry.path()));
            }
        }
    }
    out.sort();
    Ok(out)
}

fn load_device_id(dir: &Path) -> Result<DeviceId> {
    #[derive(Serialize, Deserialize)]
    struct DeviceFile {
        device_id: DeviceId,
    }
    let path = dir.join(LOCAL).join("device.json");
    if let Some(bytes) = read_opt(&path)? {
        let f: DeviceFile = serde_json::from_slice(&bytes)
            .map_err(|e| VaultError::Corrupt(format!("device.json: {e}")))?;
        return Ok(f.device_id);
    }
    let f = DeviceFile {
        device_id: DeviceId::new(),
    };
    atomic_write(&path, &serde_json::to_vec(&f)?)?;
    Ok(f.device_id)
}

impl Unlocked {
    fn seal_item(&self, file_id: &str, plaintext: &[u8]) -> Result<Vec<u8>> {
        let mut out = vec![FORMAT_VERSION as u8];
        out.extend(self.key.seal(&item_aad(file_id), plaintext)?);
        Ok(out)
    }

    /// Decrypt and validate an item file.
    fn open_item(&self, file_id: &str, bytes: &[u8]) -> Result<Entry> {
        match bytes.first() {
            Some(&v) if v as u32 == FORMAT_VERSION => {}
            Some(&v) => return Err(VaultError::UnsupportedVersion(v as u32)),
            None => return Err(VaultError::Corrupt("empty item file".into())),
        }
        let plaintext = self.key.open(&item_aad(file_id), &bytes[1..])?;
        let env: Envelope = serde_json::from_slice(&plaintext)
            .map_err(|e| VaultError::Corrupt(format!("envelope: {e}")))?;
        if self.key.file_id(&env.item_type, &env.id) != file_id {
            return Err(VaultError::Corrupt("file id does not match item".into()));
        }
        Ok(Entry {
            item_type: env.item_type,
            id: env.id,
            updated_at: env.updated_at,
            deleted: env.deleted,
            plaintext,
        })
    }
}

impl Vault {
    /// Initialize a new vault in `dir`. Returns it unlocked plus the new Secret Key, which the
    /// caller must show to the user once (Emergency Kit).
    pub fn create(
        dir: impl AsRef<Path>,
        password: &str,
        params: KdfParams,
    ) -> Result<(Vault, SecretKey)> {
        let dir = dir.as_ref().to_path_buf();
        if dir.join(MANIFEST).exists() {
            return Err(VaultError::AlreadyInitialized);
        }
        let secret_key = SecretKey::generate()?;
        let salt = random_hex(16)?;
        let mut manifest = Manifest {
            format_version: FORMAT_VERSION,
            vault_id: random_hex(16)?,
            kdf_algorithm: "argon2id".into(),
            kdf: params,
            salt,
            key_check: String::new(),
        };
        let salt_bytes = hex::decode(&manifest.salt).expect("hex we just made");
        let key = VaultKey::derive(password, &secret_key, &salt_bytes, params)?;
        manifest.key_check = hex::encode(key.seal(&manifest.key_check_aad(), KEY_CHECK_PLAINTEXT)?);
        std::fs::create_dir_all(dir.join(ITEMS))?;
        atomic_write(&dir.join(MANIFEST), &serde_json::to_vec_pretty(&manifest)?)?;
        let device = load_device_id(&dir)?;
        let vault = Vault {
            dir,
            manifest: Some(manifest),
            clock: HlcClock::new(device),
            unlocked: Some(Unlocked {
                key,
                index: HashMap::new(),
                corrupt: Vec::new(),
            }),
            revision: 0,
        };
        Ok((vault, secret_key))
    }

    /// Open the vault in `dir` (locked). An uninitialized directory yields
    /// `status().initialized == false`.
    pub fn open(dir: impl AsRef<Path>) -> Result<Vault> {
        let dir = dir.as_ref().to_path_buf();
        let manifest = read_opt(&dir.join(MANIFEST))?
            .map(|b| Manifest::parse(&b))
            .transpose()?;
        let device = load_device_id(&dir)?;
        Ok(Vault {
            dir,
            manifest,
            clock: HlcClock::new(device),
            unlocked: None,
            revision: 0,
        })
    }

    /// Set up this device for an existing vault from its synced `manifest.json` bytes.
    /// Item files then arrive via [`Vault::merge_remote`] after unlock.
    pub fn join(dir: impl AsRef<Path>, manifest_bytes: &[u8]) -> Result<Vault> {
        let dir = dir.as_ref();
        if dir.join(MANIFEST).exists() {
            return Err(VaultError::AlreadyInitialized);
        }
        Manifest::parse(manifest_bytes)?;
        std::fs::create_dir_all(dir.join(ITEMS))?;
        atomic_write(&dir.join(MANIFEST), manifest_bytes)?;
        Self::open(dir)
    }

    pub fn status(&self) -> VaultStatus {
        VaultStatus {
            initialized: self.manifest.is_some(),
            locked: self.unlocked.is_none(),
        }
    }

    pub fn vault_id(&self) -> Option<&str> {
        self.manifest.as_ref().map(|m| m.vault_id.as_str())
    }

    pub fn device_id(&self) -> DeviceId {
        self.clock.device()
    }

    /// A counter that changes whenever item files are written, merged, removed or restored in
    /// this process. Lets a background syncer notice local edits without hashing files.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Derive the vault key and decrypt all items into the in-memory index. Item files that
    /// fail authentication are skipped and reported by [`Vault::corrupt_files`].
    pub fn unlock(&mut self, password: &str, secret_key: &SecretKey) -> Result<()> {
        let manifest = self.manifest.as_ref().ok_or(VaultError::NotInitialized)?;
        let salt =
            hex::decode(&manifest.salt).map_err(|_| VaultError::Corrupt("manifest salt".into()))?;
        let check = hex::decode(&manifest.key_check)
            .map_err(|_| VaultError::Corrupt("manifest key check".into()))?;
        let key = VaultKey::derive(password, secret_key, &salt, manifest.kdf)?;
        match key.open(&manifest.key_check_aad(), &check) {
            Ok(pt) if pt.as_slice() == KEY_CHECK_PLAINTEXT => {}
            _ => return Err(VaultError::WrongCredentials),
        }
        let mut unlocked = Unlocked {
            key,
            index: HashMap::new(),
            corrupt: Vec::new(),
        };
        for (file_id, path) in sorted_dir(&self.dir.join(ITEMS), ".enc")? {
            let bytes = std::fs::read(&path)?;
            let opened = if is_file_id(&file_id) {
                unlocked.open_item(&file_id, &bytes)
            } else {
                Err(VaultError::Corrupt("bad file name".into()))
            };
            match opened {
                Ok(entry) => {
                    self.clock.observe(&entry.updated_at);
                    unlocked.index.insert(file_id, entry);
                }
                Err(_) => unlocked.corrupt.push(format!("{ITEMS}/{file_id}.enc")),
            }
        }
        self.unlocked = Some(unlocked);
        Ok(())
    }

    /// Drop the key and the decrypted index (zeroized on drop).
    pub fn lock(&mut self) {
        self.unlocked = None;
    }

    /// Delete the vault from this device: items, snapshots, manifest and device-local state.
    /// Works locked (a forgotten password is the main reason to do it). Afterwards the vault is
    /// uninitialized and [`Vault::create`] or [`Vault::join`] can start over in the same place.
    ///
    /// The directory is first renamed away in one step, so a failure part way never leaves a
    /// half-deleted vault that still opens; a copy that could not be removed is retried by the
    /// next erase.
    pub fn erase(&mut self) -> Result<()> {
        let dir = self.dir.clone();
        let parent = dir.parent().unwrap_or_else(|| Path::new("."));
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "vault".into());
        let prefix = format!(".{name}-deleted-");
        self.lock();
        let doomed = parent.join(format!("{prefix}{}", random_hex(8)?));
        match std::fs::rename(&dir, &doomed) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        std::fs::create_dir_all(&dir)?;
        *self = Vault::open(&dir)?;
        // This erase's copy and any an earlier one could not remove.
        for entry in std::fs::read_dir(parent)? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().starts_with(&prefix) {
                // Encrypted leftovers; the vault itself is already gone.
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
        Ok(())
    }

    /// Item files skipped at unlock because they failed to decrypt/validate.
    pub fn corrupt_files(&self) -> &[String] {
        self.unlocked.as_ref().map_or(&[], |u| &u.corrupt)
    }

    fn unlocked(&self) -> Result<&Unlocked> {
        self.unlocked.as_ref().ok_or(VaultError::Locked)
    }

    fn item_path(&self, file_id: &str) -> PathBuf {
        self.dir.join(ITEMS).join(format!("{file_id}.enc"))
    }

    fn write_envelope(
        &mut self,
        item_type: &str,
        id: &str,
        payload: Value,
        deleted: bool,
    ) -> Result<()> {
        let u = self.unlocked.as_ref().ok_or(VaultError::Locked)?;
        let updated_at = self.clock.now();
        let env = Envelope {
            id: id.to_string(),
            item_type: item_type.to_string(),
            updated_at,
            device_id: self.clock.device(),
            deleted,
            payload,
        };
        let plaintext = Zeroizing::new(serde_json::to_vec(&env)?);
        let file_id = u.key.file_id(item_type, id);
        let bytes = u.seal_item(&file_id, &plaintext)?;
        atomic_write(&self.item_path(&file_id), &bytes)?;
        self.revision += 1;
        let u = self.unlocked.as_mut().ok_or(VaultError::Locked)?;
        u.index.insert(
            file_id,
            Entry {
                item_type: env.item_type,
                id: env.id,
                updated_at,
                deleted,
                plaintext,
            },
        );
        Ok(())
    }

    /// Insert or replace an item.
    pub fn put<T: VaultItem>(&mut self, item: &T) -> Result<()> {
        let id = item.id();
        if id.is_empty() {
            return Err(VaultError::Invalid("empty item id".into()));
        }
        let payload = serde_json::to_value(item)?;
        self.write_envelope(T::TYPE, &id, payload, false)
    }

    pub fn get<T: VaultItem>(&self, id: &str) -> Result<Option<T>> {
        let u = self.unlocked()?;
        let file_id = u.key.file_id(T::TYPE, id);
        match u.index.get(&file_id) {
            Some(e) if !e.deleted && e.item_type == T::TYPE && e.id == id => {
                Ok(Some(serde_json::from_value(e.envelope()?.payload)?))
            }
            _ => Ok(None),
        }
    }

    /// All live items of type `T`, sorted by id.
    pub fn list<T: VaultItem>(&self) -> Result<Vec<T>> {
        let u = self.unlocked()?;
        let mut entries: Vec<&Entry> = u
            .index
            .values()
            .filter(|e| !e.deleted && e.item_type == T::TYPE)
            .collect();
        entries.sort_by(|a, b| a.id.cmp(&b.id));
        entries
            .into_iter()
            .map(|e| Ok(serde_json::from_value(e.envelope()?.payload)?))
            .collect()
    }

    /// Replace the item with a tombstone. Returns whether a live item existed.
    pub fn delete<T: VaultItem>(&mut self, id: &str) -> Result<bool> {
        let u = self.unlocked()?;
        let file_id = u.key.file_id(T::TYPE, id);
        let live = u.index.get(&file_id).is_some_and(|e| !e.deleted);
        if live {
            self.write_envelope(T::TYPE, id, Value::Null, true)?;
        }
        Ok(live)
    }

    /// Remove tombstones whose deletion is older than `older_than`. Returns how many were removed.
    pub fn compact_tombstones(&mut self, older_than: Duration) -> Result<usize> {
        let cutoff = wall_now_ms().saturating_sub(older_than.as_millis() as u64);
        let u = self.unlocked.as_ref().ok_or(VaultError::Locked)?;
        let doomed: Vec<String> = u
            .index
            .iter()
            .filter(|(_, e)| e.deleted && e.updated_at.wall_ms < cutoff)
            .map(|(fid, _)| fid.clone())
            .collect();
        for fid in &doomed {
            match std::fs::remove_file(self.item_path(fid)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
            if let Some(u) = self.unlocked.as_mut() {
                u.index.remove(fid);
            }
            self.revision += 1;
        }
        Ok(doomed.len())
    }

    // ---------------------------------------------------------------- sync

    /// The syncable files (`manifest.json` + `items/*.enc`). Works while locked.
    pub fn raw_files(&self) -> Result<Vec<RawFile>> {
        let mut out = Vec::new();
        let mut push = |path: String, bytes: Vec<u8>| {
            let sha256 = hex::encode(Sha256::digest(&bytes));
            out.push(RawFile {
                path,
                bytes,
                sha256,
            });
        };
        if let Some(b) = read_opt(&self.dir.join(MANIFEST))? {
            push(MANIFEST.to_string(), b);
        }
        for (stem, path) in sorted_dir(&self.dir.join(ITEMS), ".enc")? {
            push(format!("{ITEMS}/{stem}.enc"), std::fs::read(path)?);
        }
        Ok(out)
    }

    /// Merge one file fetched from a sync backend. Items merge last-writer-wins by HLC; the
    /// losing version goes to the local conflict history. Tampered input is rejected with
    /// `Corrupt` and nothing is written.
    pub fn merge_remote(&mut self, path: &str, bytes: &[u8]) -> Result<MergeOutcome> {
        self.merge_remote_tracked(path, bytes, false)
    }

    /// Like [`Vault::merge_remote`] for a syncer that knows whether the local file is still the
    /// version it last synced (`local_in_sync`). A newer remote version then replaces it as an
    /// ordinary pull, not a conflict: nothing was edited here, so nothing is kept in the
    /// conflict history.
    pub fn merge_remote_tracked(
        &mut self,
        path: &str,
        bytes: &[u8],
        local_in_sync: bool,
    ) -> Result<MergeOutcome> {
        let path = path.replace('\\', "/");
        if path == MANIFEST {
            return self.merge_manifest(bytes);
        }
        let file_id = path
            .strip_prefix("items/")
            .and_then(|p| p.strip_suffix(".enc"))
            .filter(|f| is_file_id(f))
            .ok_or_else(|| VaultError::Invalid(format!("not a vault file: {path}")))?
            .to_string();
        let u = self.unlocked.as_ref().ok_or(VaultError::Locked)?;
        let remote = u.open_item(&file_id, bytes)?;
        self.clock.observe(&remote.updated_at);
        let local = u.index.get(&file_id);
        let outcome = match local {
            None => MergeOutcome::Added,
            Some(l) if l.updated_at == remote.updated_at => MergeOutcome::Unchanged,
            Some(l) if remote.updated_at > l.updated_at => {
                if !local_in_sync {
                    self.record_conflict(&file_id, remote.updated_at, l.envelope()?)?;
                }
                MergeOutcome::RemoteWon
            }
            Some(l) => {
                self.record_conflict(&file_id, l.updated_at, remote.envelope()?)?;
                MergeOutcome::LocalWon
            }
        };
        if matches!(outcome, MergeOutcome::Added | MergeOutcome::RemoteWon) {
            atomic_write(&self.item_path(&file_id), bytes)?;
            self.revision += 1;
            if let Some(u) = self.unlocked.as_mut() {
                u.index.insert(file_id, remote);
            }
        }
        Ok(outcome)
    }

    fn merge_manifest(&mut self, bytes: &[u8]) -> Result<MergeOutcome> {
        let remote = Manifest::parse(bytes)?;
        match &self.manifest {
            None => {
                std::fs::create_dir_all(self.dir.join(ITEMS))?;
                atomic_write(&self.dir.join(MANIFEST), bytes)?;
                self.manifest = Some(remote);
                Ok(MergeOutcome::Added)
            }
            Some(local) if *local == remote => Ok(MergeOutcome::Unchanged),
            Some(local) if local.vault_id != remote.vault_id => Err(VaultError::Invalid(
                "remote manifest belongs to a different vault".into(),
            )),
            Some(_) => Err(VaultError::Invalid(
                "remote manifest differs (credentials changed on another device?)".into(),
            )),
        }
    }

    fn record_conflict(&self, file_id: &str, winner: Hlc, loser: Envelope) -> Result<()> {
        let u = self.unlocked()?;
        let rec = ConflictRecord {
            recorded_at_ms: wall_now_ms(),
            file_id: file_id.to_string(),
            winner,
            loser,
        };
        let plaintext = Zeroizing::new(serde_json::to_vec(&rec)?);
        let name = format!("{:015}-{}", rec.recorded_at_ms, random_hex(4)?);
        let mut bytes = vec![FORMAT_VERSION as u8];
        bytes.extend(u.key.seal(&history_aad(&name), &plaintext)?);
        atomic_write(
            &self
                .dir
                .join(LOCAL)
                .join(HISTORY)
                .join(format!("{name}.enc")),
            &bytes,
        )?;
        Ok(())
    }

    /// Versions that lost a merge, oldest first. Unreadable records are skipped.
    pub fn conflict_history(&self) -> Result<Vec<ConflictRecord>> {
        let u = self.unlocked()?;
        let mut out = Vec::new();
        for (name, path) in sorted_dir(&self.dir.join(LOCAL).join(HISTORY), ".enc")? {
            let bytes = std::fs::read(path)?;
            if bytes.first() != Some(&(FORMAT_VERSION as u8)) {
                continue;
            }
            if let Ok(pt) = u.key.open(&history_aad(&name), &bytes[1..]) {
                if let Ok(rec) = serde_json::from_slice(&pt) {
                    out.push(rec);
                }
            }
        }
        Ok(out)
    }

    /// Bring back the version a merge discarded: it is written as a fresh edit (new HLC), so it
    /// wins on the next sync. `file_id` and `recorded_at_ms` identify the [`ConflictRecord`].
    pub fn restore_conflict(&mut self, file_id: &str, recorded_at_ms: u64) -> Result<()> {
        let rec = self
            .conflict_history()?
            .into_iter()
            .rev()
            .find(|r| r.file_id == file_id && r.recorded_at_ms == recorded_at_ms)
            .ok_or_else(|| VaultError::NotFound(format!("conflict {file_id}@{recorded_at_ms}")))?;
        let l = rec.loser;
        self.write_envelope(&l.item_type, &l.id, l.payload, l.deleted)
    }

    /// Delete all local conflict history.
    pub fn clear_conflict_history(&mut self) -> Result<()> {
        match std::fs::remove_dir_all(self.dir.join(LOCAL).join(HISTORY)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    // ----------------------------------------------------------- snapshots

    /// Write an encrypted snapshot of all current item files; keeps the newest
    /// [`MAX_SNAPSHOTS`].
    pub fn snapshot(&self) -> Result<SnapshotInfo> {
        let info = self.snapshot_unpruned()?;
        self.prune_snapshots()?;
        Ok(info)
    }

    /// Like [`Vault::snapshot`] without deleting old snapshots. For a tentative snapshot (e.g.
    /// before a sync) that may be dropped again with [`Vault::delete_snapshot`]: pruning first
    /// would lose an old snapshot for nothing. Call [`Vault::prune_snapshots`] to keep it.
    pub fn snapshot_unpruned(&self) -> Result<SnapshotInfo> {
        let u = self.unlocked()?;
        let mut plain = Zeroizing::new(SNAPSHOT_MAGIC.to_vec());
        for (stem, path) in sorted_dir(&self.dir.join(ITEMS), ".enc")? {
            let data = std::fs::read(path)?;
            for chunk in [stem.as_bytes(), &data[..]] {
                plain.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
                plain.extend_from_slice(chunk);
            }
        }
        let created_ms = wall_now_ms();
        let name = format!("{created_ms:015}-{}.snap", random_hex(4)?);
        let mut bytes = vec![FORMAT_VERSION as u8];
        bytes.extend(u.key.seal(&snapshot_aad(&name), &plain)?);
        atomic_write(&self.dir.join(SNAPSHOTS).join(&name), &bytes)?;
        Ok(SnapshotInfo {
            name,
            created_ms,
            size: bytes.len() as u64,
        })
    }

    /// Delete all but the newest [`MAX_SNAPSHOTS`] snapshots.
    pub fn prune_snapshots(&self) -> Result<()> {
        let all = self.list_snapshots()?;
        if all.len() > MAX_SNAPSHOTS {
            for old in &all[..all.len() - MAX_SNAPSHOTS] {
                std::fs::remove_file(self.dir.join(SNAPSHOTS).join(&old.name))?;
            }
        }
        Ok(())
    }

    /// Delete one snapshot. Works while locked.
    pub fn delete_snapshot(&self, name: &str) -> Result<()> {
        check_snapshot_name(name)?;
        match std::fs::remove_file(self.dir.join(SNAPSHOTS).join(name)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(VaultError::NotFound(format!("snapshot {name}")))
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Snapshots on disk, oldest first. Works while locked.
    pub fn list_snapshots(&self) -> Result<Vec<SnapshotInfo>> {
        let mut out = Vec::new();
        for (stem, path) in sorted_dir(&self.dir.join(SNAPSHOTS), ".snap")? {
            let created_ms = stem
                .split('-')
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            out.push(SnapshotInfo {
                name: format!("{stem}.snap"),
                created_ms,
                size: std::fs::metadata(path)?.len(),
            });
        }
        Ok(out)
    }

    /// Roll the vault back to a snapshot. A safety snapshot of the current state is taken
    /// first. Differences are written as fresh edits (new HLC) so the rollback wins on sync;
    /// items absent from the snapshot become tombstones.
    pub fn restore_snapshot(&mut self, name: &str) -> Result<()> {
        check_snapshot_name(name)?;
        let u = self.unlocked()?;
        let bytes = read_opt(&self.dir.join(SNAPSHOTS).join(name))?
            .ok_or_else(|| VaultError::NotFound(format!("snapshot {name}")))?;
        if bytes.first() != Some(&(FORMAT_VERSION as u8)) {
            return Err(VaultError::Corrupt("snapshot header".into()));
        }
        let plain = u.key.open(&snapshot_aad(name), &bytes[1..])?;
        let mut target: HashMap<String, Envelope> = HashMap::new();
        for (file_id, data) in parse_snapshot(&plain)? {
            let entry = u.open_item(&file_id, data)?;
            target.insert(file_id, entry.envelope()?);
        }
        // Validate everything before changing anything.
        self.snapshot()?;

        let u = self.unlocked()?;
        let mut writes: Vec<(String, String, Value, bool)> = Vec::new();
        for (fid, e) in &u.index {
            if !e.deleted && !target.contains_key(fid) {
                writes.push((e.item_type.clone(), e.id.clone(), Value::Null, true));
            }
        }
        for (fid, t) in target {
            let current = u.index.get(fid.as_str()).map(Entry::envelope).transpose()?;
            let same = current
                .as_ref()
                .is_some_and(|c| c.deleted == t.deleted && (t.deleted || c.payload == t.payload));
            if !same {
                writes.push((t.item_type, t.id, t.payload, t.deleted));
            }
        }
        for (item_type, id, payload, deleted) in writes {
            self.write_envelope(&item_type, &id, payload, deleted)?;
        }
        Ok(())
    }
}

fn check_snapshot_name(name: &str) -> Result<()> {
    if name.contains(['/', '\\']) || !name.ends_with(".snap") {
        return Err(VaultError::Invalid(format!("bad snapshot name {name}")));
    }
    Ok(())
}

fn history_aad(name: &str) -> Vec<u8> {
    format!("aht-history:v{FORMAT_VERSION}:{name}").into_bytes()
}

fn snapshot_aad(name: &str) -> Vec<u8> {
    format!("aht-snapshot:v{FORMAT_VERSION}:{name}").into_bytes()
}

fn take_chunk<'a>(rest: &mut &'a [u8]) -> Result<&'a [u8]> {
    let bad = || VaultError::Corrupt("snapshot body".into());
    let len_bytes: [u8; 4] = rest
        .get(..4)
        .ok_or_else(bad)?
        .try_into()
        .map_err(|_| bad())?;
    let n = u32::from_le_bytes(len_bytes) as usize;
    let body = rest.get(4..4 + n).ok_or_else(bad)?;
    *rest = &rest[4 + n..];
    Ok(body)
}

fn parse_snapshot(plain: &[u8]) -> Result<Vec<(String, &[u8])>> {
    let bad = || VaultError::Corrupt("snapshot body".into());
    let mut rest = plain.strip_prefix(SNAPSHOT_MAGIC).ok_or_else(bad)?;
    let mut out = Vec::new();
    while !rest.is_empty() {
        let name = std::str::from_utf8(take_chunk(&mut rest)?).map_err(|_| bad())?;
        if !is_file_id(name) {
            return Err(bad());
        }
        let data = take_chunk(&mut rest)?;
        out.push((name.to_string(), data));
    }
    Ok(out)
}
