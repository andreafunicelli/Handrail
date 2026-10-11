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
    pub(crate) name: String,
    pub(crate) path: String,
    /// "dir" | "file" | "symlink" | "other"
    pub(crate) kind: &'static str,
    /// A symlink; `kind` is then its target's kind.
    pub(crate) link: bool,
    pub(crate) size: u64,
    /// Seconds since the Unix epoch.
    pub(crate) modified: Option<u32>,
    pub(crate) permissions: Option<u32>,
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
            link: e.link,
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
pub(crate) fn reporter(
    app: &AppHandle,
    transfer_id: String,
) -> impl FnMut(u64, u64) + Send + 'static {
    let app = app.clone();
    let mut last: Option<Instant> = None;
    move |done, total| {
        let due = last.is_none_or(|t| t.elapsed() >= Duration::from_millis(100));
        if due || done == total {
            last = Some(Instant::now());
            let _ = app.emit(
                "transfer://progress",
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
pub(crate) fn free_local_path(dir: &Path, name: &str) -> PathBuf {
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

/// The local file name for a remote file, valid on every OS: characters Windows forbids and
/// control characters become `_`, trailing dots and spaces go, and device names (`CON`, `COM1`…)
/// get a `_` in front.
pub(crate) fn local_name(remote: &str) -> Result<String> {
    let name = remote.rsplit('/').next().unwrap_or_default();
    if name.is_empty() || name == "." || name == ".." {
        return Err(Error::Invalid(format!("cannot download {remote}")));
    }
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_control() || r#"<>:"\|?*"#.contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    out.truncate(out.trim_end_matches(['.', ' ']).len());
    if out.is_empty() {
        out.push('_');
    }
    let stem = out.split('.').next().unwrap_or_default().trim_end();
    let stem = stem.to_ascii_uppercase();
    let numbered = |p: &str| {
        stem.len() == 4 && stem.starts_with(p) && matches!(stem.as_bytes()[3], b'1'..=b'9')
    };
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") || numbered("COM") || numbered("LPT")
    {
        out.insert(0, '_');
    }
    Ok(out)
}

/// The Downloads folder, or home when there is none.
pub(crate) fn downloads(app: &AppHandle) -> Result<PathBuf> {
    app.path()
        .download_dir()
        .or_else(|_| app.path().home_dir())
        .map_err(err)
}

/// The file name of a local path picked in the file dialog.
pub(crate) fn picked_name(local: &Path) -> Result<String> {
    local
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .ok_or_else(|| Error::Invalid("not a file".into()))
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
    let local = free_local_path(&downloads(&app)?, &local_name(&path)?);
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
    let name = picked_name(&local_path)?;
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
        assert_eq!(
            local_name("/r/report:2026?.txt").unwrap(),
            "report_2026_.txt"
        );
        assert_eq!(local_name("/r/a\\b\t<c>|\"d*").unwrap(), "a_b__c___d_");
        assert_eq!(local_name("/r/notes. . ").unwrap(), "notes");
        assert_eq!(local_name("/r/...").unwrap(), "_");
        assert_eq!(local_name("/r/con").unwrap(), "_con");
        assert_eq!(local_name("/r/COM1.log").unwrap(), "_COM1.log");
        assert_eq!(local_name("/r/nul .tar.gz").unwrap(), "_nul .tar.gz");
        assert_eq!(local_name("/r/COM0").unwrap(), "COM0");
        assert_eq!(local_name("/r/console.log").unwrap(), "console.log");
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
