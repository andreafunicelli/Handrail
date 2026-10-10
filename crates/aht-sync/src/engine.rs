//! The sync round: pull changed remote files into the vault (LWW merge), then push local files
//! that changed since the last push. The caller persists [`SyncState`] between runs.

use crate::backend::{Change, SyncBackend};
use crate::error::{Result, SyncError};
use aht_vault::{MergeOutcome, RawFile, Vault, VaultError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};

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

/// One full pull + push. Idempotent: with no changes on either side it only lists the remote.
/// A concurrent writer (etag conflict / non-fast-forward) restarts the round, up to a few times.
pub async fn sync_once(
    vault: &mut Vault,
    backend: &dyn SyncBackend,
    state: &mut SyncState,
) -> Result<SyncReport> {
    if !vault.status().initialized {
        return Err(SyncError::LocalNotInitialized);
    }
    let mut report = SyncReport::default();
    let mut result = Err(SyncError::TooManyRetries);
    for _ in 0..MAX_ROUNDS {
        match round(vault, backend, state, &mut report).await {
            Err(SyncError::Conflict(_)) => continue,
            other => {
                result = other;
                break;
            }
        }
    }
    report.warnings.extend(backend.take_warnings());
    result.map(|()| report)
}

fn is_item_path(p: &str) -> bool {
    p.strip_prefix("items/")
        .is_some_and(|r| r.ends_with(".enc") && !r.contains('/'))
}

async fn round(
    vault: &mut Vault,
    backend: &dyn SyncBackend,
    state: &mut SyncState,
    report: &mut SyncReport,
) -> Result<()> {
    let remote = backend.list().await?;
    let remote_etag: HashMap<&str, &str> = remote
        .iter()
        .map(|e| (e.path.as_str(), e.etag.as_str()))
        .collect();
    let before: HashMap<String, String> = vault
        .raw_files()?
        .into_iter()
        .map(|f| (f.path, f.sha256))
        .collect();

    // ---- manifest: must match; never re-keyed in v1.
    if let Some(&etag) = remote_etag.get(MANIFEST) {
        let seen = state.remote_etags.get(MANIFEST).map(String::as_str) == Some(etag)
            && state.pushed_sha.contains_key(MANIFEST);
        if !seen {
            let bytes = backend.get(MANIFEST).await?;
            match vault.merge_remote(MANIFEST, &bytes) {
                Ok(_) => {}
                Err(VaultError::Invalid(_)) => return Err(SyncError::ManifestMismatch),
                Err(e) => return Err(e.into()),
            }
            state.remote_etags.insert(MANIFEST.into(), etag.into());
            state.pushed_sha.insert(MANIFEST.into(), sha(&bytes));
        }
    }

    // ---- pull
    for entry in remote.iter().filter(|e| is_item_path(&e.path)) {
        if state.remote_etags.get(&entry.path) == Some(&entry.etag) {
            continue;
        }
        let bytes = backend.get(&entry.path).await?;
        // Unchanged here since the last sync: a newer remote version is a plain pull.
        let local_in_sync = before
            .get(&entry.path)
            .is_some_and(|sha| state.pushed_sha.get(&entry.path) == Some(sha));
        let outcome = match vault.merge_remote_tracked(&entry.path, &bytes, local_in_sync) {
            Ok(o) => o,
            Err(VaultError::Corrupt(why)) => {
                report
                    .warnings
                    .push(format!("skipped corrupt remote file {}: {why}", entry.path));
                continue;
            }
            Err(e) => return Err(e.into()),
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

    // ---- push (re-read: merges may have changed files)
    let local: Vec<RawFile> = vault.raw_files()?;
    let changes: Vec<Change> = local
        .iter()
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
    let by_path: HashMap<&str, &RawFile> = local.iter().map(|f| (f.path.as_str(), f)).collect();
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
        vault: Vault,
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
                vault,
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
            vault,
            state: SyncState::default(),
        }
    }

    async fn sync(d: &mut Device, b: &dyn SyncBackend) -> SyncReport {
        sync_once(&mut d.vault, b, &mut d.state).await.unwrap()
    }

    #[tokio::test]
    async fn create_on_a_appears_on_b_and_noop_second_sync() {
        let remote = tempfile::tempdir().unwrap();
        let be = FolderBackend::new(remote.path());
        let (mut a, sk) = device_a();
        a.vault.put(&setting("x", 1)).unwrap();
        a.vault.put(&setting("y", 2)).unwrap();
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
        a.vault.put(&setting("k", 1)).unwrap();
        sync(&mut a, &be).await;
        let mut b = device_b(&be, &sk).await;
        sync(&mut b, &be).await;
        assert_eq!(val(&b, "k"), Some(1));

        a.vault.put(&setting("k", 2)).unwrap();
        sync(&mut a, &be).await;
        let rb = sync(&mut b, &be).await;
        assert_eq!((rb.pulled, rb.conflicts), (1, 0));
        assert_eq!(val(&b, "k"), Some(2));
        assert!(
            b.vault.conflict_history().unwrap().is_empty(),
            "B never edited k: nothing to bring back"
        );
    }

    #[tokio::test]
    async fn concurrent_edit_is_lww_with_conflict_history() {
        let remote = tempfile::tempdir().unwrap();
        let be = FolderBackend::new(remote.path());
        let (mut a, sk) = device_a();
        a.vault.put(&setting("k", 1)).unwrap();
        sync(&mut a, &be).await;
        let mut b = device_b(&be, &sk).await;
        sync(&mut b, &be).await;

        a.vault.put(&setting("k", 2)).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        b.vault.put(&setting("k", 3)).unwrap(); // later edit wins
        sync(&mut a, &be).await;
        let rb = sync(&mut b, &be).await;
        assert_eq!(rb.conflicts, 1);
        assert_eq!(rb.pushed, 1, "winner must be pushed back");
        assert_eq!(b.vault.conflict_history().unwrap().len(), 1);
        assert_eq!(val(&b, "k"), Some(3));
        let ra = sync(&mut a, &be).await;
        assert_eq!(val(&a, "k"), Some(3));
        assert_eq!(ra.pulled, 1);
        // A's 2 had already been synced; losing to B's newer 3 is an ordinary pull on A.
        assert!(a.vault.conflict_history().unwrap().is_empty());
        assert_eq!(sync(&mut b, &be).await, SyncReport::default());
        assert_eq!(sync(&mut a, &be).await, SyncReport::default());
    }

    #[tokio::test]
    async fn delete_propagates() {
        let remote = tempfile::tempdir().unwrap();
        let be = FolderBackend::new(remote.path());
        let (mut a, sk) = device_a();
        a.vault.put(&setting("k", 1)).unwrap();
        sync(&mut a, &be).await;
        let mut b = device_b(&be, &sk).await;
        sync(&mut b, &be).await;
        assert_eq!(val(&b, "k"), Some(1));
        a.vault.delete::<Setting>("k").unwrap();
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
        let err = sync_once(&mut other.vault, &be, &mut other.state)
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
        a.vault.put(&setting("k", 1)).unwrap();
        let r = sync(&mut a, &be).await;
        assert_eq!(r.pushed, 2);
        assert_eq!(be.writes.load(Ordering::SeqCst), 1);

        let be2 = Flaky {
            inner: FolderBackend::new(remote.path()),
            fail: AtomicUsize::new(100),
            writes: AtomicUsize::new(0),
        };
        a.vault.put(&setting("k", 2)).unwrap();
        let err = sync_once(&mut a.vault, &be2, &mut a.state)
            .await
            .unwrap_err();
        assert!(matches!(err, SyncError::TooManyRetries), "{err:?}");
    }
}
