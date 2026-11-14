use crate::backend::{RemoteEntry, SyncBackend};
use crate::error::{Result, SyncError};
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

/// Mirror of the vault files in a plain folder (typically inside iCloud/Dropbox/Syncthing).
/// etag = SHA-256 hex of the content.
pub struct FolderBackend {
    root: PathBuf,
    warnings: Mutex<Vec<String>>,
}

enum Skip {
    No,
    /// Silently ignored (temp files, dotfiles).
    Quiet,
    /// Ignored, with a warning.
    Warn(String),
}

fn classify(name: &str) -> Skip {
    let lower = name.to_lowercase();
    if lower.contains(".sync-conflict-") {
        Skip::Warn(format!("ignored Syncthing conflict file: {name}"))
    } else if lower.contains("(conflicted copy") {
        Skip::Warn(format!("ignored Dropbox conflict file: {name}"))
    } else if lower.ends_with(".icloud") {
        Skip::Warn(format!(
            "iCloud has not downloaded this file yet, ignored: {name}"
        ))
    } else if name.starts_with('.')
        || lower.ends_with(".tmp")
        || lower.ends_with(".part")
        || lower.ends_with(".crdownload")
        || lower.ends_with('~')
    {
        Skip::Quiet
    } else {
        Skip::No
    }
}

fn etag_of(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Run blocking file I/O off the async runtime: the folder may sit on a slow or stalled network
/// or cloud drive.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| SyncError::Io(std::io::Error::other(e.to_string())))?
}

fn current_etag(full: &Path) -> Result<Option<String>> {
    match std::fs::read(full) {
        Ok(b) => Ok(Some(etag_of(&b))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn check(path: &str, full: &Path, expected: Option<&str>) -> Result<()> {
    if current_etag(full)?.as_deref() != expected {
        return Err(SyncError::Conflict(path.to_string()));
    }
    Ok(())
}

fn walk(
    dir: &Path,
    prefix: &str,
    out: &mut Vec<RemoteEntry>,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    for entry in rd {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        match classify(&name) {
            Skip::Quiet => continue,
            Skip::Warn(w) => {
                warnings.push(w);
                continue;
            }
            Skip::No => {}
        }
        let rel = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        if entry.file_type()?.is_dir() {
            walk(&entry.path(), &rel, out, warnings)?;
        } else {
            let etag = etag_of(&std::fs::read(entry.path())?);
            out.push(RemoteEntry { path: rel, etag });
        }
    }
    Ok(())
}

impl FolderBackend {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            warnings: Mutex::new(Vec::new()),
        }
    }

    fn resolve(&self, path: &str) -> Result<PathBuf> {
        let rel = Path::new(path);
        let bad = rel.as_os_str().is_empty()
            || rel.components().any(|c| !matches!(c, Component::Normal(_)));
        if bad || path.contains('\\') {
            return Err(SyncError::InvalidPath(path.to_string()));
        }
        Ok(self.root.join(rel))
    }
}

#[async_trait]
impl SyncBackend for FolderBackend {
    async fn list(&self) -> Result<Vec<RemoteEntry>> {
        let root = self.root.clone();
        let (mut out, warnings) = blocking(move || {
            let (mut out, mut warnings) = (Vec::new(), Vec::new());
            walk(&root, "", &mut out, &mut warnings)?;
            Ok((out, warnings))
        })
        .await?;
        self.warnings.lock().unwrap().extend(warnings);
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }

    async fn get(&self, path: &str) -> Result<Vec<u8>> {
        let full = self.resolve(path)?;
        let path = path.to_string();
        blocking(move || match std::fs::read(full) {
            Ok(b) => Ok(b),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(SyncError::NotFound(path)),
            Err(e) => Err(e.into()),
        })
        .await
    }

    async fn put(&self, path: &str, bytes: &[u8], expected: Option<&str>) -> Result<String> {
        let full = self.resolve(path)?;
        let (path, bytes) = (path.to_string(), bytes.to_vec());
        let expected = expected.map(str::to_owned);
        blocking(move || {
            check(&path, &full, expected.as_deref())?;
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let tmp = full.with_file_name(format!(
                ".aht-tmp-{}-{}",
                std::process::id(),
                full.file_name().unwrap_or_default().to_string_lossy()
            ));
            std::fs::write(&tmp, &bytes)?;
            if let Err(e) = std::fs::rename(&tmp, &full) {
                let _ = std::fs::remove_file(&tmp);
                return Err(e.into());
            }
            Ok(etag_of(&bytes))
        })
        .await
    }

    async fn delete(&self, path: &str, expected: Option<&str>) -> Result<()> {
        let full = self.resolve(path)?;
        let (path, expected) = (path.to_string(), expected.map(str::to_owned));
        blocking(move || {
            check(&path, &full, expected.as_deref())?;
            match std::fs::remove_file(&full) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
                _ => Ok(()),
            }
        })
        .await
    }

    fn take_warnings(&self) -> Vec<String> {
        let mut w = self.warnings.lock().unwrap();
        w.sort();
        w.dedup();
        std::mem::take(&mut *w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn put_get_list_delete() {
        let d = tempfile::tempdir().unwrap();
        let b = FolderBackend::new(d.path());
        assert!(b.list().await.unwrap().is_empty());
        let e = b.put("items/a.enc", b"hello", None).await.unwrap();
        assert_eq!(b.get("items/a.enc").await.unwrap(), b"hello");
        let l = b.list().await.unwrap();
        assert_eq!(
            l,
            vec![RemoteEntry {
                path: "items/a.enc".into(),
                etag: e.clone()
            }]
        );
        let e2 = b.put("items/a.enc", b"world", Some(&e)).await.unwrap();
        assert_ne!(e, e2);
        b.delete("items/a.enc", Some(&e2)).await.unwrap();
        assert!(b.list().await.unwrap().is_empty());
        assert!(matches!(
            b.get("items/a.enc").await,
            Err(SyncError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn etag_mismatch_is_conflict() {
        let d = tempfile::tempdir().unwrap();
        let b = FolderBackend::new(d.path());
        let e = b.put("x.enc", b"1", None).await.unwrap();
        assert!(matches!(
            b.put("x.enc", b"2", None).await,
            Err(SyncError::Conflict(_))
        ));
        assert!(matches!(
            b.put("x.enc", b"2", Some("deadbeef")).await,
            Err(SyncError::Conflict(_))
        ));
        assert!(matches!(
            b.delete("x.enc", Some("deadbeef")).await,
            Err(SyncError::Conflict(_))
        ));
        assert_eq!(b.get("x.enc").await.unwrap(), b"1");
        b.put("x.enc", b"2", Some(&e)).await.unwrap();
    }

    #[tokio::test]
    async fn ignores_temp_and_conflict_files_with_warnings() {
        let d = tempfile::tempdir().unwrap();
        let b = FolderBackend::new(d.path());
        b.put("items/a.enc", b"1", None).await.unwrap();
        let items = d.path().join("items");
        for n in [
            "a.sync-conflict-20260101-120000-ABCDEF.enc",
            "a (conflicted copy 2026-01-01).enc",
            ".a.enc.icloud",
            ".aht-tmp-1-a.enc",
            "b.enc.tmp",
        ] {
            std::fs::write(items.join(n), b"junk").unwrap();
        }
        let l = b.list().await.unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].path, "items/a.enc");
        let w = b.take_warnings();
        assert_eq!(w.len(), 3, "{w:?}");
        assert!(b.take_warnings().is_empty());
    }

    #[tokio::test]
    async fn rejects_path_traversal() {
        let d = tempfile::tempdir().unwrap();
        let b = FolderBackend::new(d.path());
        for p in ["../x", "/abs", "a/../../x", "", "a\\b"] {
            assert!(matches!(
                b.put(p, b"1", None).await,
                Err(SyncError::InvalidPath(_))
            ));
        }
    }
}
