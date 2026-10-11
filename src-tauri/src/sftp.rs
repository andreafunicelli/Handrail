//! File browser commands: SFTP on the SSH connection of an open terminal session, so browsing
//! files needs no second login. Downloads go to the Downloads folder; uploads come from paths
//! the user picked in the native file dialog.

use crate::state::{err, AppState};
use aht_core::{Error, Result, SessionId};
use aht_ssh::{EntryKind, RemoteEntry, Sftp};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Serialize)]
pub struct FileEntry {
    name: String,
    path: String,
    /// "dir" | "file" | "symlink" | "other"
    kind: &'static str,
    size: u64,
    /// Seconds since the Unix epoch.
    modified: Option<u32>,
    permissions: Option<u32>,
}

impl From<RemoteEntry> for FileEntry {
    fn from(e: RemoteEntry) -> Self {
        FileEntry {
            name: e.name,
            path: e.path,
            kind: match e.kind {
                EntryKind::Dir => "dir",
                EntryKind::File => "file",
                EntryKind::Symlink => "symlink",
                EntryKind::Other => "other",
            },
            size: e.size,
            modified: e.modified,
            permissions: e.permissions,
        }
    }
}

#[derive(Clone, Serialize)]
struct ProgressEvent {
    transfer_id: String,
    done: u64,
    total: u64,
}

/// The session's SFTP client, started on first use.
async fn sftp(state: &AppState, session_id: SessionId) -> Result<Arc<Sftp>> {
    let mut map = state.terminal.sftp.lock().await;
    if let Some(s) = map.get(&session_id) {
        return Ok(s.clone());
    }
    let conn = state.terminal.ssh_connection(session_id).await?;
    let s = Arc::new(conn.open_sftp().await?);
    map.insert(session_id, s.clone());
    Ok(s)
}

/// Progress events for `transfer_id`, at most ten a second plus the final one.
fn reporter(app: &AppHandle, transfer_id: String) -> impl FnMut(u64, u64) + Send {
    let app = app.clone();
    let mut last: Option<Instant> = None;
    move |done, total| {
        let due = last.is_none_or(|t| t.elapsed() >= Duration::from_millis(100));
        if due || done == total {
            last = Some(Instant::now());
            let _ = app.emit(
                "sftp://progress",
                ProgressEvent {
                    transfer_id: transfer_id.clone(),
                    done,
                    total,
                },
            );
        }
    }
}

/// The login directory of the session's user.
#[tauri::command(rename_all = "snake_case")]
pub async fn sftp_home(state: State<'_, AppState>, session_id: SessionId) -> Result<String> {
    sftp(&state, session_id).await?.canonicalize(".").await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn sftp_list(
    state: State<'_, AppState>,
    session_id: SessionId,
    path: String,
) -> Result<Vec<FileEntry>> {
    let s = sftp(&state, session_id).await?;
    Ok(s.list(&path).await?.into_iter().map(Into::into).collect())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn sftp_mkdir(
    state: State<'_, AppState>,
    session_id: SessionId,
    path: String,
) -> Result<()> {
    sftp(&state, session_id).await?.mkdir(&path).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn sftp_rename(
    state: State<'_, AppState>,
    session_id: SessionId,
    from: String,
    to: String,
) -> Result<()> {
    sftp(&state, session_id).await?.rename(&from, &to).await
}

/// Delete a file or an empty folder.
#[tauri::command(rename_all = "snake_case")]
pub async fn sftp_remove(
    state: State<'_, AppState>,
    session_id: SessionId,
    path: String,
    is_dir: bool,
) -> Result<()> {
    sftp(&state, session_id).await?.remove(&path, is_dir).await
}

/// `dir/name`, or `dir/stem (2).ext`… when taken.
fn free_local_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .expect("an unused name")
}

/// A remote file name safe to use as a local file name.
fn local_name(remote: &str) -> Result<String> {
    let name = remote.rsplit('/').next().unwrap_or_default();
    if name.is_empty() || name == "." || name == ".." || name.contains(['\\', '\0']) {
        return Err(Error::Invalid(format!("cannot download {remote}")));
    }
    Ok(name.to_string())
}

/// Download a remote file into the Downloads folder; returns the local path.
#[tauri::command(rename_all = "snake_case")]
pub async fn sftp_download(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: SessionId,
    path: String,
    transfer_id: String,
) -> Result<String> {
    let s = sftp(&state, session_id).await?;
    let dir = app
        .path()
        .download_dir()
        .or_else(|_| app.path().home_dir())
        .map_err(err)?;
    let local = free_local_path(&dir, &local_name(&path)?);
    s.download(&path, &local, reporter(&app, transfer_id))
        .await?;
    Ok(local.display().to_string())
}

/// Upload a local file (picked in the file dialog) into `dir`; returns the remote path.
#[tauri::command(rename_all = "snake_case")]
pub async fn sftp_upload(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: SessionId,
    local_path: PathBuf,
    dir: String,
    transfer_id: String,
) -> Result<String> {
    let name = local_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| Error::Invalid("not a file".into()))?
        .to_string();
    let s = sftp(&state, session_id).await?;
    let remote = aht_ssh::sftp::join(&dir, &name);
    s.upload(&local_path, &remote, reporter(&app, transfer_id))
        .await?;
    Ok(remote)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_names_and_free_paths() {
        assert_eq!(local_name("/var/log/syslog").unwrap(), "syslog");
        assert!(local_name("/var/log/").is_err());
        assert!(local_name("/tmp/..").is_err());
        let dir = std::env::temp_dir().join(format!("aht-sftp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(free_local_path(&dir, "a.tar.gz"), dir.join("a.tar.gz"));
        std::fs::write(dir.join("a.tar.gz"), b"").unwrap();
        assert_eq!(free_local_path(&dir, "a.tar.gz"), dir.join("a.tar (2).gz"));
        std::fs::write(dir.join("notes"), b"").unwrap();
        assert_eq!(free_local_path(&dir, "notes"), dir.join("notes (2)"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
