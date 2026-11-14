//! The sync round: pull changed remote files into the vault (LWW merge), then push local files
//! that changed since the last push. The caller persists [`SyncState`] between runs.
//!
//! The vault is locked only for the short local steps (reading its files, one merge), never
//! across network or remote-folder I/O, so a stalled backend does not block the app's vault
//! commands (lock included). A local edit between steps restarts the round; a vault locked or
//! replaced meanwhile ends it.

use crate::backend::{Change, SyncBackend};
use crate::error::{Result, SyncError};
use aht_vault::{is_item_path, MergeOutcome, RawFile, Vault, VaultError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use tokio::sync::Mutex;

const MANIFEST: &str = "manifest.json";
const MAX_ROUNDS: usize = 4;

/// Persisted by the caller (serde) between syncs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncState {
    /// Remote etag per path as of the last time we pulled or pushed it.
    pub remote_etags: BTreeMap<String, String>,
    /// SHA-256 of the local file per path as of the last time it was in sync with the remote.
    pub pushed_sha: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncReport {
    /// Remote files merged into the vault (new or cleanly updated).
    pub pulled: usize,
    /// Local files written to the remote.
    pub pushed: usize,
    /// Merges where both sides had changed the item (loser kept in the local conflict history).
    pub conflicts: usize,
    pub warnings: Vec<String>,
    /// [`Vault::revision`] the remote is now in line with: a later revision is a local edit
    /// still to push. Not persisted.
    #[serde(skip)]
    pub revision: u64,
}

fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// New-device onboarding: download the remote `manifest.json`, to be passed to `Vault::join`.
pub async fn fetch_manifest(backend: &dyn SyncBackend) -> Result<Vec<u8>> {
    if !backend.list().await?.iter().any(|e| e.path == MANIFEST) {
        return Err(SyncError::NoRemoteVault);
    }
    backend.get(MANIFEST).await
}

/// The vault as the round sees it: locked for one short step at a time.
struct Local<'a> {
    vault: &'a Mutex<Vault>,
    vault_id: Option<String>,
    /// Revision after our last step; anything else in between was an edit by someone else.
    revision: u64,
    /// The automatic snapshot taken before this sync's first merge (once tried, `Some`).
    snapshot: Option<Option<String>>,
    /// A merge changed local items: the snapshot is worth keeping.
    changed: bool,
}

impl Local<'_> {
    /// Run `f` on the unlocked vault. With `fresh`, local edits since the last step are fine
    /// (the step reads the current state); otherwise they restart the round.
    async fn with<R>(&mut self, fresh: bool, f: impl FnOnce(&mut Vault) -> Result<R>) -> Result<R> {
        let mut v = self.vault.lock().await;
        if v.status().locked || v.vault_id() != self.vault_id.as_deref() {
            return Err(SyncError::Vault(VaultError::Locked));
        }
        if !fresh && v.revision() != self.revision {
            return Err(SyncError::Conflict(
                "the vault was edited during the sync".into(),
            ));
        }
        let r = f(&mut v);
        self.revision = v.revision();
        r
    }

    /// The syncable files and the paths among them that do not open.
    async fn files(&mut self) -> Result<(Vec<RawFile>, HashSet<String>)> {
        self.with(true, |v| {
            Ok((v.raw_files()?, v.corrupt_files().iter().cloned().collect()))
        })
        .await
    }

    /// Merge one remote item file; the sync's first merge snapshots the vault just before.
    async fn merge(
        &mut self,
        path: &str,
        bytes: &[u8],
        in_sync: bool,
        report: &mut SyncReport,
    ) -> Result<MergeOutcome> {
        let first = self.snapshot.is_none();
        let (outcome, snap) = self
            .with(false, |v| {
                let snap = first.then(|| v.snapshot_auto());
                Ok((v.merge_remote_tracked(path, bytes, in_sync), snap))
            })
            .await?;
        if let Some(snap) = snap {
            // A rollback point is a safety net: syncing without one beats not syncing.
            self.snapshot = Some(match snap {
                Ok(info) => Some(info.name),
                Err(e) => {
                    report
                        .warnings
                        .push(format!("could not snapshot the vault before syncing: {e}"));
                    None
                }
            });
        }
        let outcome = outcome?;
        self.changed |= matches!(outcome, MergeOutcome::Added | MergeOutcome::RemoteWon);
        Ok(outcome)
    }

    /// Keep the pre-sync snapshot if the sync changed local items, else drop it again.
    async fn tidy_snapshot(&mut self, report: &mut SyncReport) {
        let Some(Some(name)) = self.snapshot.take() else {
            return;
        };
        let v = self.vault.lock().await;
        if v.vault_id() != self.vault_id.as_deref() {
            return;
        }
        let tidied = if self.changed {
            v.prune_snapshots()
        } else {
            v.delete_snapshot(&name)
        };
        if let Err(e) = tidied {
            report
                .warnings
                .push(format!("could not tidy the pre-sync snapshot: {e}"));
        }
    }
}

/// One full pull + push. Idempotent: with no changes on either side it only lists the remote.
/// A concurrent writer (etag conflict / non-fast-forward), or a local edit made while the round
/// was talking to the remote, restarts the round, up to a few times. Before the first merge
/// that may change local items, an automatic snapshot is taken (kept only if one did).
pub async fn sync_once(
    vault: &Mutex<Vault>,
    backend: &dyn SyncBackend,
    state: &mut SyncState,
) -> Result<SyncReport> {
    let mut local = {
        let v = vault.lock().await;
        if !v.status().initialized {
            return Err(SyncError::LocalNotInitialized);
        }
        if v.status().locked {
            return Err(SyncError::Vault(VaultError::Locked));
        }
        Local {
            vault,
            vault_id: v.vault_id().map(str::to_owned),
            revision: v.revision(),
            snapshot: None,
            changed: false,
        }
    };
    let mut report = SyncReport::default();
    let mut result = Err(SyncError::TooManyRetries);
    for _ in 0..MAX_ROUNDS {
        match round(&mut local, backend, state, &mut report).await {
            Err(SyncError::Conflict(_)) => continue,
            other => {
                result = other;
                break;
            }
        }
    }
    local.tidy_snapshot(&mut report).await;
    report.warnings.extend(backend.take_warnings());
    // Rounds that restarted repeat their warnings.
    report.warnings.sort();
    report.warnings.dedup();
    report.revision = local.revision;
    result.map(|()| report)
}

async fn round(
    local: &mut Local<'_>,
    backend: &dyn SyncBackend,
    state: &mut SyncState,
    report: &mut SyncReport,
) -> Result<()> {
    let remote = backend.list().await?;
    let remote_etag: HashMap<&str, &str> = remote
        .iter()
        .map(|e| (e.path.as_str(), e.etag.as_str()))
        .collect();
    let (files, corrupt) = local.files().await?;
    let before: HashMap<String, String> = files.into_iter().map(|f| (f.path, f.sha256)).collect();
    for path in &corrupt {
        report.warnings.push(format!(
            "local file {path} does not open and is not uploaded"
        ));
    }

    // ---- manifest: must match; never re-keyed in v1.
    if let Some(&etag) = remote_etag.get(MANIFEST) {
        let seen = state.remote_etags.get(MANIFEST).map(String::as_str) == Some(etag)
            && state.pushed_sha.contains_key(MANIFEST);
        if !seen {
            let bytes = backend.get(MANIFEST).await?;
            match local
                .with(true, |v| Ok(v.merge_remote(MANIFEST, &bytes)))
                .await?
            {
                Ok(_) => {}
                Err(VaultError::Invalid(_)) => return Err(SyncError::ManifestMismatch),
                Err(e) => return Err(e.into()),
            }
            state.remote_etags.insert(MANIFEST.into(), etag.into());
            state.pushed_sha.insert(MANIFEST.into(), sha(&bytes));
        }
    }

    // ---- pull
    for entry in remote.iter().filter(|e| e.path.starts_with("items/")) {
        if !is_item_path(&entry.path) {
            // E.g. a cloud drive's "<id> (1).enc" conflict copy: not a vault file.
            report
                .warnings
                .push(format!("ignored unexpected remote file {}", entry.path));
            continue;
        }
        let local_corrupt = corrupt.contains(&entry.path);
        if !local_corrupt && state.remote_etags.get(&entry.path) == Some(&entry.etag) {
            continue;
        }
        let bytes = backend.get(&entry.path).await?;
        // Unchanged here since the last sync: a newer remote version is a plain pull.
        let local_in_sync = before
            .get(&entry.path)
            .is_some_and(|sha| state.pushed_sha.get(&entry.path) == Some(sha));
        let outcome = match local
            .merge(&entry.path, &bytes, local_in_sync, report)
            .await
        {
            Ok(o) => o,
            Err(SyncError::Vault(VaultError::Corrupt(why))) => {
                report
                    .warnings
                    .push(format!("skipped corrupt remote file {}: {why}", entry.path));
                // A good local copy goes back up over it.
                if before.contains_key(&entry.path) && !local_corrupt {
                    state.pushed_sha.remove(&entry.path);
                }
                continue;
            }
            Err(e) => return Err(e),
        };
        let local_before = before.get(&entry.path);
        match outcome {
            MergeOutcome::Added => {
                report.pulled += 1;
                state.pushed_sha.insert(entry.path.clone(), sha(&bytes));
            }
            MergeOutcome::RemoteWon => {
                if local_before == state.pushed_sha.get(&entry.path) {
                    report.pulled += 1;
                } else {
                    report.conflicts += 1;
                }
                state.pushed_sha.insert(entry.path.clone(), sha(&bytes));
            }
            MergeOutcome::LocalWon => {
                report.conflicts += 1;
                // Local is newer than what the remote has: make sure it gets pushed.
                state.pushed_sha.remove(&entry.path);
            }
            MergeOutcome::Unchanged => {
                if let Some(l) = local_before {
                    state.pushed_sha.insert(entry.path.clone(), l.clone());
                }
            }
        }
        state
            .remote_etags
            .insert(entry.path.clone(), entry.etag.clone());
    }

    // ---- push (re-read: merges may have changed files). Never a file that does not open:
    // that would replace the good remote copy with a broken one.
    let (local_files, corrupt) = local.files().await?;
    let changes: Vec<Change> = local_files
        .iter()
        .filter(|f| !corrupt.contains(&f.path))
        .filter(|f| {
            state.pushed_sha.get(&f.path) != Some(&f.sha256)
                || !remote_etag.contains_key(f.path.as_str())
        })
        .map(|f| Change {
            path: f.path.clone(),
            bytes: f.bytes.clone(),
            expected: remote_etag.get(f.path.as_str()).map(|s| s.to_string()),
        })
        .collect();
    if changes.is_empty() {
        return Ok(());
    }
    let written = backend.put_batch(changes).await?;
    let by_path: HashMap<&str, &RawFile> =
        local_files.iter().map(|f| (f.path.as_str(), f)).collect();
    for w in written {
        if let Some(f) = by_path.get(w.path.as_str()) {
            state.pushed_sha.insert(w.path.clone(), f.sha256.clone());
        }
        state.remote_etags.insert(w.path, w.etag);
        report.pushed += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::RemoteEntry;
    use crate::FolderBackend;
    use aht_vault::{KdfParams, SecretKey, Setting};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Device {
        _dir: tempfile::TempDir,
        vault: Mutex<Vault>,
        state: SyncState,
    }

    fn setting(k: &str, v: i64) -> Setting {
        Setting {
            key: k.into(),
            value: serde_json::json!(v),
        }
    }

    fn val(d: &Device, k: &str) -> Option<i64> {
        d.vault
            .try_lock()
            .unwrap()
            .get::<Setting>(k)
            .unwrap()
            .map(|s| s.value.as_i64().unwrap())
    }

    fn device_a() -> (Device, SecretKey) {
        let dir = tempfile::tempdir().unwrap();
        let (vault, sk) = Vault::create(dir.path(), "pw", KdfParams::insecure_fast()).unwrap();
        (
            Device {
                _dir: dir,
                vault: Mutex::new(vault),
                state: SyncState::default(),
            },
            sk,
        )
    }

    async fn device_b(backend: &dyn SyncBackend, sk: &SecretKey) -> Device {
        let dir = tempfile::tempdir().unwrap();
        let m = fetch_manifest(backend).await.unwrap();
        let mut vault = Vault::join(dir.path(), &m).unwrap();
        vault.unlock("pw", sk).unwrap();
        Device {
            _dir: dir,
            vault: Mutex::new(vault),
            state: SyncState::default(),
        }
    }

    async fn sync(d: &mut Device, b: &dyn SyncBackend) -> SyncReport {
        let mut r = sync_once(&d.vault, b, &mut d.state).await.unwrap();
        r.revision = 0; // compared with `SyncReport::default()` for no-op syncs
        r
    }

    #[tokio::test]
    async fn create_on_a_appears_on_b_and_noop_second_sync() {
        let remote = tempfile::tempdir().unwrap();
        let be = FolderBackend::new(remote.path());
        let (mut a, sk) = device_a();
        a.vault.get_mut().put(&setting("x", 1)).unwrap();
        a.vault.get_mut().put(&setting("y", 2)).unwrap();
        let r = sync(&mut a, &be).await;
        assert_eq!(r.pushed, 3); // manifest + 2 items
        let mut b = device_b(&be, &sk).await;
        let r = sync(&mut b, &be).await;
        assert_eq!((r.pulled, r.pushed, r.conflicts), (2, 0, 0));
        assert_eq!(val(&b, "x"), Some(1));
        assert_eq!(val(&b, "y"), Some(2));
        // no-op
        assert_eq!(sync(&mut a, &be).await, SyncReport::default());
        assert_eq!(sync(&mut b, &be).await, SyncReport::default());
    }

    #[tokio::test]
    async fn clean_pull_of_a_newer_edit_is_not_a_conflict() {
        let remote = tempfile::tempdir().unwrap();
        let be = FolderBackend::new(remote.path());
        let (mut a, sk) = device_a();
        a.vault.get_mut().put(&setting("k", 1)).unwrap();
        sync(&mut a, &be).await;
        let mut b = device_b(&be, &sk).await;
        sync(&mut b, &be).await;
        assert_eq!(val(&b, "k"), Some(1));

        a.vault.get_mut().put(&setting("k", 2)).unwrap();
        sync(&mut a, &be).await;
        let rb = sync(&mut b, &be).await;
        assert_eq!((rb.pulled, rb.conflicts), (1, 0));
        assert_eq!(val(&b, "k"), Some(2));
        assert!(
            b.vault.get_mut().conflict_history().unwrap().is_empty(),
            "B never edited k: nothing to bring back"
        );
    }

    #[tokio::test]
    async fn concurrent_edit_is_lww_with_conflict_history() {
        let remote = tempfile::tempdir().unwrap();
        let be = FolderBackend::new(remote.path());
        let (mut a, sk) = device_a();
        a.vault.get_mut().put(&setting("k", 1)).unwrap();
        sync(&mut a, &be).await;
        let mut b = device_b(&be, &sk).await;
        sync(&mut b, &be).await;

        a.vault.get_mut().put(&setting("k", 2)).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        b.vault.get_mut().put(&setting("k", 3)).unwrap(); // later edit wins
        sync(&mut a, &be).await;
        let rb = sync(&mut b, &be).await;
        assert_eq!(rb.conflicts, 1);
        assert_eq!(rb.pushed, 1, "winner must be pushed back");
        assert_eq!(b.vault.get_mut().conflict_history().unwrap().len(), 1);
        assert_eq!(val(&b, "k"), Some(3));
        let ra = sync(&mut a, &be).await;
        assert_eq!(val(&a, "k"), Some(3));
        assert_eq!(ra.pulled, 1);
        // A's 2 had already been synced; losing to B's newer 3 is an ordinary pull on A.
        assert!(a.vault.get_mut().conflict_history().unwrap().is_empty());
        assert_eq!(sync(&mut b, &be).await, SyncReport::default());
        assert_eq!(sync(&mut a, &be).await, SyncReport::default());
    }

    #[tokio::test]
    async fn delete_propagates() {
        let remote = tempfile::tempdir().unwrap();
        let be = FolderBackend::new(remote.path());
        let (mut a, sk) = device_a();
        a.vault.get_mut().put(&setting("k", 1)).unwrap();
        sync(&mut a, &be).await;
        let mut b = device_b(&be, &sk).await;
        sync(&mut b, &be).await;
        assert_eq!(val(&b, "k"), Some(1));
        a.vault.get_mut().delete::<Setting>("k").unwrap();
        sync(&mut a, &be).await;
        sync(&mut b, &be).await;
        assert_eq!(val(&b, "k"), None);
    }

    #[tokio::test]
    async fn manifest_mismatch_is_error() {
        let remote = tempfile::tempdir().unwrap();
        let be = FolderBackend::new(remote.path());
        let (mut a, _) = device_a();
        sync(&mut a, &be).await;
        let (mut other, _) = device_a();
        let err = sync_once(&other.vault, &be, &mut other.state)
            .await
            .unwrap_err();
        assert!(matches!(err, SyncError::ManifestMismatch), "{err:?}");
    }

    #[tokio::test]
    async fn fetch_manifest_on_empty_remote() {
        let remote = tempfile::tempdir().unwrap();
        let be = FolderBackend::new(remote.path());
        assert!(matches!(
            fetch_manifest(&be).await,
            Err(SyncError::NoRemoteVault)
        ));
    }

    /// Fails the first `put_batch` with a conflict (simulating a racing device).
    struct Flaky {
        inner: FolderBackend,
        fail: AtomicUsize,
        writes: AtomicUsize,
    }
    #[async_trait]
    impl SyncBackend for Flaky {
        async fn list(&self) -> Result<Vec<RemoteEntry>> {
            self.inner.list().await
        }
        async fn get(&self, p: &str) -> Result<Vec<u8>> {
            self.inner.get(p).await
        }
        async fn put(&self, p: &str, b: &[u8], e: Option<&str>) -> Result<String> {
            self.inner.put(p, b, e).await
        }
        async fn delete(&self, p: &str, e: Option<&str>) -> Result<()> {
            self.inner.delete(p, e).await
        }
        async fn put_batch(&self, c: Vec<Change>) -> Result<Vec<RemoteEntry>> {
            if self
                .fail
                .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok()
            {
                return Err(SyncError::Conflict("race".into()));
            }
            self.writes.fetch_add(1, Ordering::SeqCst);
            let mut out = Vec::new();
            for ch in c {
                let etag = self
                    .inner
                    .put(&ch.path, &ch.bytes, ch.expected.as_deref())
                    .await?;
                out.push(RemoteEntry {
                    path: ch.path,
                    etag,
                });
            }
            Ok(out)
        }
    }

    #[tokio::test]
    async fn conflict_restarts_the_round() {
        let remote = tempfile::tempdir().unwrap();
        let be = Flaky {
            inner: FolderBackend::new(remote.path()),
            fail: AtomicUsize::new(1),
            writes: AtomicUsize::new(0),
        };
        let (mut a, _) = device_a();
        a.vault.get_mut().put(&setting("k", 1)).unwrap();
        let r = sync(&mut a, &be).await;
        assert_eq!(r.pushed, 2);
        assert_eq!(be.writes.load(Ordering::SeqCst), 1);

        let be2 = Flaky {
            inner: FolderBackend::new(remote.path()),
            fail: AtomicUsize::new(100),
            writes: AtomicUsize::new(0),
        };
        a.vault.get_mut().put(&setting("k", 2)).unwrap();
        let err = sync_once(&a.vault, &be2, &mut a.state).await.unwrap_err();
        assert!(matches!(err, SyncError::TooManyRetries), "{err:?}");
    }

    fn snapshots(d: &mut Device) -> Vec<aht_vault::SnapshotInfo> {
        d.vault.get_mut().list_snapshots().unwrap()
    }

    fn item_path(d: &mut Device) -> String {
        let files = d.vault.get_mut().raw_files().unwrap();
        files
            .into_iter()
            .find(|f| is_item_path(&f.path))
            .unwrap()
            .path
    }

    #[tokio::test]
    async fn stray_files_are_skipped_on_both_sides() {
        let remote = tempfile::tempdir().unwrap();
        let be = FolderBackend::new(remote.path());
        let (mut a, sk) = device_a();
        a.vault.get_mut().put(&setting("k", 1)).unwrap();
        sync(&mut a, &be).await;
        let item = item_path(&mut a);
        // A cloud drive's conflict copy on the remote, a stray file locally on B.
        let copy = remote.path().join(item.replace(".enc", " (1).enc"));
        std::fs::copy(remote.path().join(&item), &copy).unwrap();
        let mut b = device_b(&be, &sk).await;
        std::fs::write(b._dir.path().join("items/notes.enc"), b"junk").unwrap();
        b.vault.get_mut().lock();
        b.vault.get_mut().unlock("pw", &sk).unwrap();

        let r = sync(&mut b, &be).await;
        assert_eq!((r.pulled, r.pushed), (1, 0));
        assert_eq!(val(&b, "k"), Some(1));
        assert!(r.warnings.iter().any(|w| w.contains(" (1).enc")), "{r:?}");
        assert!(r.warnings.iter().any(|w| w.contains("notes.enc")), "{r:?}");
        assert!(!remote.path().join("items/notes.enc").exists());
        // Still in step: the next sync has nothing to do but warn again.
        let r = sync(&mut b, &be).await;
        assert_eq!((r.pulled, r.pushed, r.conflicts), (0, 0, 0));
    }

    #[tokio::test]
    async fn a_corrupt_local_file_is_not_pushed_and_is_healed() {
        let remote = tempfile::tempdir().unwrap();
        let be = FolderBackend::new(remote.path());
        let (mut a, sk) = device_a();
        a.vault.get_mut().put(&setting("k", 1)).unwrap();
        sync(&mut a, &be).await;
        let item = item_path(&mut a);
        let good = std::fs::read(remote.path().join(&item)).unwrap();
        let local = a._dir.path().join(&item);
        let mut bad = good.clone();
        bad[20] ^= 1;
        std::fs::write(&local, &bad).unwrap();
        a.vault.get_mut().lock();
        a.vault.get_mut().unlock("pw", &sk).unwrap();
        assert_eq!(val(&a, "k"), None);

        let r = sync(&mut a, &be).await;
        assert_eq!((r.pulled, r.pushed), (1, 0), "{r:?}");
        assert_eq!(std::fs::read(remote.path().join(&item)).unwrap(), good);
        assert_eq!(std::fs::read(&local).unwrap(), good);
        assert_eq!(val(&a, "k"), Some(1));
        assert!(a.vault.get_mut().corrupt_files().is_empty());
    }

    #[tokio::test]
    async fn a_corrupt_remote_file_is_replaced_by_the_good_local_copy() {
        let remote = tempfile::tempdir().unwrap();
        let be = FolderBackend::new(remote.path());
        let (mut a, _) = device_a();
        a.vault.get_mut().put(&setting("k", 1)).unwrap();
        sync(&mut a, &be).await;
        let item = item_path(&mut a);
        let good = std::fs::read(a._dir.path().join(&item)).unwrap();
        std::fs::write(remote.path().join(&item), b"\x01broken").unwrap();

        let r = sync(&mut a, &be).await;
        assert_eq!(r.pushed, 1, "{r:?}");
        assert!(
            r.warnings.iter().any(|w| w.contains("corrupt remote")),
            "{r:?}"
        );
        assert_eq!(std::fs::read(remote.path().join(&item)).unwrap(), good);
        assert_eq!(sync(&mut a, &be).await, SyncReport::default());
    }

    #[tokio::test]
    async fn pre_sync_snapshot_only_when_a_merge_changes_local_items() {
        let remote = tempfile::tempdir().unwrap();
        let be = FolderBackend::new(remote.path());
        let (mut a, sk) = device_a();
        a.vault.get_mut().put(&setting("k", 1)).unwrap();
        a.vault.get_mut().snapshot().unwrap();
        sync(&mut a, &be).await;
        assert_eq!(snapshots(&mut a).len(), 1, "a push alone takes none");
        let mut b = device_b(&be, &sk).await;
        sync(&mut b, &be).await;
        let s = snapshots(&mut b);
        assert_eq!(s.len(), 1);
        assert!(s[0].auto);
        sync(&mut b, &be).await;
        assert_eq!(snapshots(&mut b).len(), 1, "a no-op sync takes none");
        // Pulling something that is already here (Unchanged) keeps none either.
        b.state.remote_etags.clear();
        sync(&mut b, &be).await;
        assert_eq!(snapshots(&mut b).len(), 1);
        // Many pulls never evict a manual snapshot on A.
        for n in 2..15 {
            b.vault.get_mut().put(&setting("k", n)).unwrap();
            sync(&mut b, &be).await;
            sync(&mut a, &be).await;
        }
        let s = snapshots(&mut a);
        assert_eq!(s.iter().filter(|s| !s.auto).count(), 1);
        assert_eq!(
            s.iter().filter(|s| s.auto).count(),
            aht_vault::MAX_SNAPSHOTS
        );
    }

    /// Runs `hook` on every `get`, while the round is between local steps.
    struct Hooked<F> {
        inner: FolderBackend,
        hook: F,
    }
    #[async_trait]
    impl<F: Fn() + Send + Sync> SyncBackend for Hooked<F> {
        async fn list(&self) -> Result<Vec<RemoteEntry>> {
            self.inner.list().await
        }
        async fn get(&self, p: &str) -> Result<Vec<u8>> {
            (self.hook)();
            self.inner.get(p).await
        }
        async fn put(&self, p: &str, b: &[u8], e: Option<&str>) -> Result<String> {
            self.inner.put(p, b, e).await
        }
        async fn delete(&self, p: &str, e: Option<&str>) -> Result<()> {
            self.inner.delete(p, e).await
        }
    }

    #[tokio::test]
    async fn the_vault_is_free_during_remote_io_and_edits_restart_the_round() {
        let remote = tempfile::tempdir().unwrap();
        let be = FolderBackend::new(remote.path());
        let (mut a, sk) = device_a();
        a.vault.get_mut().put(&setting("k", 1)).unwrap();
        sync(&mut a, &be).await;
        let b = device_b(&be, &sk).await;
        let vault = std::sync::Arc::new(b.vault);
        let gets = std::sync::Arc::new(AtomicUsize::new(0));
        let (v2, g2) = (vault.clone(), gets.clone());
        let be = Hooked {
            inner: FolderBackend::new(remote.path()),
            hook: move || {
                let mut v = v2
                    .try_lock()
                    .expect("vault must not be held during remote I/O");
                // A local edit while the item is fetched (gets: manifest, item): the merge must
                // not use the state read before it.
                if g2.fetch_add(1, Ordering::SeqCst) == 1 {
                    v.put(&setting("local", 9)).unwrap();
                }
            },
        };
        let mut state = SyncState::default();
        let r = sync_once(&vault, &be, &mut state).await.unwrap();
        let v = vault.lock().await;
        assert_eq!(r.revision, v.revision());
        assert_eq!(v.get::<Setting>("k").unwrap().unwrap().value, 1);
        assert_eq!(r.pushed, 1, "the local edit went up");
        assert_eq!(
            gets.load(Ordering::SeqCst),
            3,
            "the round restarted after the edit"
        );
        assert!(remote.path().join("items").read_dir().unwrap().count() == 2);
    }

    #[tokio::test]
    async fn locking_the_vault_mid_sync_ends_it() {
        let remote = tempfile::tempdir().unwrap();
        let be = FolderBackend::new(remote.path());
        let (mut a, sk) = device_a();
        a.vault.get_mut().put(&setting("k", 1)).unwrap();
        sync(&mut a, &be).await;
        let b = device_b(&be, &sk).await;
        let vault = std::sync::Arc::new(b.vault);
        let v2 = vault.clone();
        let be = Hooked {
            inner: FolderBackend::new(remote.path()),
            hook: move || v2.try_lock().unwrap().lock(),
        };
        let err = sync_once(&vault, &be, &mut SyncState::default())
            .await
            .unwrap_err();
        assert!(
            matches!(err, SyncError::Vault(VaultError::Locked)),
            "{err:?}"
        );
        assert!(vault.lock().await.status().locked);
    }
}
