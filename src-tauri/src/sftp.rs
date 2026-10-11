//! File browser commands: SFTP on the SSH connection of an open terminal session, so browsing
//! files needs no second login. Downloads go to the Downloads folder; uploads come from paths
//! the user picked in the native file dialog.

use crate::cmd_vault::secret_value;
use crate::state::{err, AppState};
use aht_core::model::{Connection, SshAuth};
use aht_core::{Error, Result, SessionId};
use aht_ssh::sftp::SUDO_PASSWORD_REQUIRED;
use aht_ssh::{EntryKind, FileStamp, RemoteEntry, Sftp};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};
use zeroize::Zeroizing;

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

/// The session's SFTP client: as the login user (started on first use), or as root once
/// [`sftp_sudo`] started it.
pub(crate) async fn client(
    state: &AppState,
    session_id: SessionId,
    root: Option<bool>,
) -> Result<Arc<Sftp>> {
    let root = root.unwrap_or(false);
    let mut map = state.terminal.sftp.lock().await;
    if let Some(s) = map.get(&(session_id, root)) {
        return Ok(s.clone());
    }
    if root {
        return Err(Error::Invalid(
            "switch the file browser to root first".into(),
        ));
    }
    let conn = state.terminal.ssh_connection(session_id).await?;
    let s = Arc::new(conn.open_sftp().await?);
    map.insert((session_id, false), s.clone());
    Ok(s)
}

/// The saved login password of the session's connection, if it logs in with one.
async fn login_password(state: &AppState, session_id: SessionId) -> Option<Zeroizing<String>> {
    let conn_id = state
        .terminal
        .session(session_id)
        .await
        .ok()?
        .info()
        .connection_id?;
    let conn = state
        .vault
        .lock()
        .await
        .get::<Connection>(&conn_id.to_string())
        .ok()??;
    match conn.auth {
        SshAuth::Password { secret } => secret_value(state, secret).await.ok().map(Zeroizing::new),
        _ => None,
    }
}

/// Start the session's root SFTP server through sudo. Without `password`: passwordless sudo,
/// then the connection's saved login password; fails with `sudo_password_required` when the
/// user has to type it. The password is used once and never stored.
#[tauri::command(rename_all = "snake_case")]
pub async fn sftp_sudo(
    state: State<'_, AppState>,
    session_id: SessionId,
    password: Option<String>,
) -> Result<()> {
    if state
        .terminal
        .sftp
        .lock()
        .await
        .contains_key(&(session_id, true))
    {
        return Ok(());
    }
    let password = password.map(Zeroizing::new);
    let conn = state.terminal.ssh_connection(session_id).await?;
    let needs_password = |e: &Error| matches!(e, Error::Auth(m) if m == SUDO_PASSWORD_REQUIRED);
    let s = match password {
        Some(pw) => conn.open_sftp_sudo(Some(pw.as_str())).await?,
        None => match conn.open_sftp_sudo(None).await {
            Ok(s) => s,
            Err(e) if needs_password(&e) => match login_password(&state, session_id).await {
                // A different sudo password: ask for it instead of reporting a wrong one.
                Some(pw) => conn
                    .open_sftp_sudo(Some(pw.as_str()))
                    .await
                    .map_err(|_| Error::Auth(SUDO_PASSWORD_REQUIRED.into()))?,
                None => return Err(e),
            },
            Err(e) => return Err(e),
        },
    };
    state
        .terminal
        .sftp
        .lock()
        .await
        .insert((session_id, true), Arc::new(s));
    Ok(())
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
pub async fn sftp_home(
    state: State<'_, AppState>,
    session_id: SessionId,
    root: Option<bool>,
) -> Result<String> {
    client(&state, session_id, root)
        .await?
        .canonicalize(".")
        .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn sftp_list(
    state: State<'_, AppState>,
    session_id: SessionId,
    root: Option<bool>,
    path: String,
) -> Result<Vec<FileEntry>> {
    let s = client(&state, session_id, root).await?;
    Ok(s.list(&path).await?.into_iter().map(Into::into).collect())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn sftp_mkdir(
    state: State<'_, AppState>,
    session_id: SessionId,
    root: Option<bool>,
    path: String,
) -> Result<()> {
    client(&state, session_id, root).await?.mkdir(&path).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn sftp_rename(
    state: State<'_, AppState>,
    session_id: SessionId,
    root: Option<bool>,
    from: String,
    to: String,
) -> Result<()> {
    client(&state, session_id, root)
        .await?
        .rename(&from, &to)
        .await
}

/// Delete a file or an empty folder.
#[tauri::command(rename_all = "snake_case")]
pub async fn sftp_remove(
    state: State<'_, AppState>,
    session_id: SessionId,
    root: Option<bool>,
    path: String,
    is_dir: bool,
) -> Result<()> {
    client(&state, session_id, root)
        .await?
        .remove(&path, is_dir)
        .await
}

/// Largest file the editor opens.
const MAX_TEXT: u64 = 5 * 1024 * 1024;
/// Error text for a file changed on the server since it was read.
pub(crate) const CHANGED_ON_SERVER: &str = "changed_on_server";

/// What a file looked like when read (size and mtime), sent back with the save.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamp {
    size: u64,
    modified: Option<u32>,
}

impl From<FileStamp> for Stamp {
    fn from(s: FileStamp) -> Self {
        Stamp {
            size: s.size,
            modified: s.modified,
        }
    }
}

#[derive(Serialize)]
pub struct TextFile {
    pub(crate) text: String,
    pub(crate) stamp: Stamp,
}

/// Bytes as editable text: UTF-8 without NUL bytes (those mean a binary file).
fn as_text(path: &str, bytes: Vec<u8>) -> Result<String> {
    if bytes.contains(&0) {
        return Err(Error::Invalid(format!("{path} is a binary file")));
    }
    String::from_utf8(bytes).map_err(|_| Error::Invalid(format!("{path} is not UTF-8 text")))
}

pub(crate) async fn read_text(s: &Sftp, path: &str) -> Result<TextFile> {
    let stamp = s.stamp(path).await?.into();
    let text = as_text(path, s.read_all(path, MAX_TEXT).await?)?;
    Ok(TextFile { text, stamp })
}

/// Open a text file for the editor.
#[tauri::command(rename_all = "snake_case")]
pub async fn sftp_read_text(
    state: State<'_, AppState>,
    session_id: SessionId,
    root: Option<bool>,
    path: String,
) -> Result<TextFile> {
    read_text(&*client(&state, session_id, root).await?, &path).await
}

/// Save editor text in place. `expected` is the stamp from when the file was read (`None` for
/// a new file): if the file changed since, the save fails with `changed_on_server` instead of
/// overwriting. `backup` first copies the current contents to `<path>.bak`.
#[tauri::command(rename_all = "snake_case")]
pub async fn sftp_write_text(
    state: State<'_, AppState>,
    session_id: SessionId,
    root: Option<bool>,
    path: String,
    text: String,
    expected: Option<Stamp>,
    backup: bool,
) -> Result<Stamp> {
    let s = client(&state, session_id, root).await?;
    let current: Option<Stamp> = s.stamp(&path).await.ok().map(Into::into);
    if current != expected {
        return Err(Error::Busy(CHANGED_ON_SERVER.into()));
    }
    if backup && current.is_some() {
        let old = s.read_all(&path, MAX_TEXT).await?;
        s.write_all(&format!("{path}.bak"), &old).await?;
    }
    s.write_all(&path, text.as_bytes()).await?;
    Ok(s.stamp(&path).await?.into())
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
    root: Option<bool>,
    path: String,
    transfer_id: String,
) -> Result<String> {
    let s = client(&state, session_id, root).await?;
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
    root: Option<bool>,
    local_path: PathBuf,
    dir: String,
    transfer_id: String,
) -> Result<String> {
    let name = picked_name(&local_path)?;
    let s = client(&state, session_id, root).await?;
    let remote = aht_ssh::sftp::join(&dir, &name);
    s.upload(&local_path, &remote, reporter(&app, transfer_id))
        .await?;
    Ok(remote)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_text_opens() {
        assert_eq!(as_text("a", b"x = 1\n".to_vec()).unwrap(), "x = 1\n");
        assert!(as_text("a", vec![0x7f, b'E', b'L', b'F', 0, 1]).is_err());
        assert!(as_text("a", vec![0xff, 0xfe, b'a']).is_err());
    }

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
