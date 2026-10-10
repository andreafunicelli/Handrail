use crate::error::Result;
use async_trait::async_trait;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteEntry {
    pub path: String,
    pub etag: String,
}

/// One file write in a batch. `expected` is the etag the writer last saw (`None` = the file must
/// not exist yet); a mismatch fails the whole batch with `SyncError::Conflict`.
#[derive(Clone, Debug)]
pub struct Change {
    pub path: String,
    pub bytes: Vec<u8>,
    pub expected: Option<String>,
}

#[async_trait]
pub trait SyncBackend: Send + Sync {
    async fn list(&self) -> Result<Vec<RemoteEntry>>;
    async fn get(&self, path: &str) -> Result<Vec<u8>>;
    async fn put(&self, path: &str, bytes: &[u8], expected: Option<&str>) -> Result<String>;
    async fn delete(&self, path: &str, expected: Option<&str>) -> Result<()>;

    /// Write several files; returns the new entries. Backends that can do this atomically
    /// (GitHub: one commit) override it; the default writes one by one.
    async fn put_batch(&self, changes: Vec<Change>) -> Result<Vec<RemoteEntry>> {
        let mut out = Vec::new();
        for c in changes {
            let etag = self.put(&c.path, &c.bytes, c.expected.as_deref()).await?;
            out.push(RemoteEntry { path: c.path, etag });
        }
        Ok(out)
    }

    /// Non-fatal problems noticed since the last call (e.g. sync-conflict copies in a folder).
    fn take_warnings(&self) -> Vec<String> {
        Vec::new()
    }
}
