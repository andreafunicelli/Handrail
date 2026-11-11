//! "This computer" in the file manager: the local files, as a pane beside a server or S3.
//!
//! The page works with slash paths everywhere, so local paths cross the boundary in that form:
//! the same path on macOS and Linux, and `/C:/Users/me` for `C:\Users\me` on Windows, whose
//! root `/` lists the drives. Paths from the page must be absolute and contain no `..`.

use crate::sftp::{as_text, FileEntry, Stamp, TextFile, MAX_TEXT};
use aht_core::{Error, Result};
use aht_ssh::FileStamp;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use tauri::{AppHandle, Manager};

/// A page path as a path on this computer. `windows` selects the Windows form (`/C:/x`);
/// on Windows `/` itself is the drive list and has no path (`None`).
fn native_in(ui: &str, windows: bool) -> Result<Option<PathBuf>> {
    let bad = || Error::Invalid(format!("not a local path: {ui}"));
    if !ui.starts_with('/') || ui.contains('\0') {
        return Err(bad());
    }
    let parts: Vec<&str> = ui
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    if parts.contains(&"..") {
        return Err(bad());
    }
    if !windows {
        return Ok(Some(PathBuf::from(format!("/{}", parts.join("/")))));
    }
    let Some((drive, rest)) = parts.split_first() else {
        return Ok(None);
    };
    let d = drive.as_bytes();
    if d.len() != 2
        || !d[0].is_ascii_alphabetic()
        || d[1] != b':'
        || rest.iter().any(|p| p.contains('\\'))
    {
        return Err(bad());
    }
    let mut s = format!("{}:\\", d[0].to_ascii_uppercase() as char);
    s.push_str(&rest.join("\\"));
    Ok(Some(PathBuf::from(s)))
}

/// A path on this computer in the page's slash form (the inverse of `native_in`).
fn ui_in(native: &str, windows: bool) -> String {
    if !windows {
        return native.to_string();
    }
    let s = native.replace('\\', "/");
    let s = s.trim_end_matches('/');
    format!("/{s}")
}

const WINDOWS: bool = cfg!(windows);

fn native(ui: &str) -> Result<PathBuf> {
    native_in(ui, WINDOWS)?.ok_or_else(|| Error::Invalid("pick a drive first".into()))
}

pub(crate) fn ui_path(p: &Path) -> String {
    ui_in(&p.to_string_lossy(), WINDOWS)
}

/// Join a slash folder path and a name, as the page does.
fn join_ui(dir: &str, name: &str) -> String {
    format!("{}/{name}", dir.trim_end_matches('/'))
}

fn io(path: &str) -> impl Fn(std::io::Error) -> Error + '_ {
    move |e| match e.kind() {
        std::io::ErrorKind::NotFound => Error::NotFound(path.to_string()),
        std::io::ErrorKind::AlreadyExists => Error::Busy(aht_ssh::sftp::EXISTS.into()),
        _ => Error::Other(format!("{path}: {e}")),
    }
}

fn secs(m: &std::fs::Metadata) -> Option<u32> {
    m.modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| u32::try_from(d.as_secs()).ok())
}

#[cfg(unix)]
fn mode(m: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    Some(m.mode())
}
#[cfg(not(unix))]
fn mode(_: &std::fs::Metadata) -> Option<u32> {
    None
}

/// Size, time and mode of a local file (following links), for the comparison before a copy.
pub(crate) fn stamp(p: &Path) -> Option<FileStamp> {
    let m = std::fs::metadata(p).ok()?;
    Some(FileStamp {
        size: m.len(),
        modified: secs(&m),
        permissions: mode(&m),
    })
}

fn entry(dir: &str, name: String, path: &Path) -> FileEntry {
    let link = std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink());
    let (kind, size, modified, permissions) = match std::fs::metadata(path) {
        Ok(m) => {
            let kind = if m.is_dir() {
                "dir"
            } else if m.is_file() {
                "file"
            } else {
                "other"
            };
            (
                kind,
                if m.is_file() { m.len() } else { 0 },
                secs(&m),
                mode(&m),
            )
        }
        // A link whose target is missing.
        Err(_) => ("symlink", 0, None, None),
    };
    FileEntry {
        path: join_ui(dir, &name),
        name,
        kind,
        link,
        size,
        modified,
        permissions,
    }
}

/// The home folder.
#[tauri::command(rename_all = "snake_case")]
pub fn local_home(app: AppHandle) -> Result<String> {
    let home = app
        .path()
        .home_dir()
        .map_err(|e| Error::Other(e.to_string()))?;
    Ok(ui_path(&home))
}

/// The entries of a folder (on Windows, `/` lists the drives).
#[tauri::command(rename_all = "snake_case")]
pub async fn local_list(path: String) -> Result<Vec<FileEntry>> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Vec<FileEntry>> {
        let Some(dir) = native_in(&path, WINDOWS)? else {
            return Ok(drives());
        };
        let mut out = Vec::new();
        for item in std::fs::read_dir(&dir).map_err(io(&path))? {
            let item = item.map_err(io(&path))?;
            let name = item.file_name().to_string_lossy().into_owned();
            out.push(entry(&path, name, &item.path()));
        }
        Ok(out)
    })
    .await
    .map_err(|e| Error::Other(e.to_string()))?
}

/// The drives that exist (Windows); nowhere else is `/` without a path.
fn drives() -> Vec<FileEntry> {
    (b'A'..=b'Z')
        .filter_map(|d| {
            let root = format!("{}:\\", d as char);
            Path::new(&root).exists().then(|| FileEntry {
                name: format!("{}:", d as char),
                path: format!("/{}:", d as char),
                kind: "dir",
                link: false,
                size: 0,
                modified: None,
                permissions: None,
            })
        })
        .collect()
}

#[tauri::command(rename_all = "snake_case")]
pub fn local_mkdir(path: String) -> Result<()> {
    std::fs::create_dir(native(&path)?).map_err(io(&path))
}

/// Rename or move inside this computer. Never replaces anything: a taken name is `exists`.
#[tauri::command(rename_all = "snake_case")]
pub fn local_rename(from: String, to: String) -> Result<()> {
    let (src, dst) = (native(&from)?, native(&to)?);
    if std::fs::symlink_metadata(&dst).is_ok() {
        return Err(Error::Busy(aht_ssh::sftp::EXISTS.into()));
    }
    std::fs::rename(src, dst).map_err(io(&from))
}

/// Delete a file, a link (never what it points to), or a folder with what it holds.
#[tauri::command(rename_all = "snake_case")]
pub async fn local_remove(path: String) -> Result<()> {
    tauri::async_runtime::spawn_blocking(move || -> Result<()> {
        let p = native(&path)?;
        if p.parent().is_none() {
            return Err(Error::Invalid(format!("{path} cannot be deleted")));
        }
        let m = std::fs::symlink_metadata(&p).map_err(io(&path))?;
        let r = if m.is_dir() {
            std::fs::remove_dir_all(&p)
        } else {
            // A file, or a link (a directory link on Windows needs remove_dir).
            std::fs::remove_file(&p).or_else(|e| {
                if m.file_type().is_symlink() {
                    std::fs::remove_dir(&p)
                } else {
                    Err(e)
                }
            })
        };
        r.map_err(io(&path))
    })
    .await
    .map_err(|e| Error::Other(e.to_string()))?
}

/// A local text file, for the comparison before a copy replaces it.
#[tauri::command(rename_all = "snake_case")]
pub async fn local_read_text(path: String) -> Result<TextFile> {
    let p = native(&path)?;
    let m = tokio::fs::metadata(&p).await.map_err(io(&path))?;
    if m.len() > MAX_TEXT {
        return Err(Error::Invalid(format!("{path} is too large to show")));
    }
    let bytes = tokio::fs::read(&p).await.map_err(io(&path))?;
    let info = stamp(&p).unwrap_or(FileStamp {
        size: m.len(),
        modified: None,
        permissions: None,
    });
    Ok(TextFile {
        stamp: Stamp::of(info, &bytes),
        text: as_text(&path, bytes)?,
    })
}

/// A regular local file to copy from.
pub(crate) fn source_file(ui: &str) -> Result<PathBuf> {
    let p = native(ui)?;
    let m = std::fs::metadata(&p).map_err(io(ui))?;
    if !m.is_file() {
        return Err(Error::Invalid(format!(
            "{ui} is a folder: folders are copied between servers only"
        )));
    }
    Ok(p)
}

/// Where a copy named `name` lands in the local folder `dir`: its path, and a free temporary
/// path beside it to write first. `exists` unless `overwrite` when the name is taken.
pub(crate) fn dest_file(
    dir: &str,
    name: &str,
    overwrite: bool,
) -> Result<(PathBuf, PathBuf, String)> {
    let ui = join_ui(dir, name);
    let dest = native(&ui)?;
    if let Ok(m) = std::fs::symlink_metadata(&dest) {
        if !overwrite {
            return Err(Error::Busy(aht_ssh::sftp::EXISTS.into()));
        }
        if m.is_dir() {
            return Err(Error::Invalid(format!(
                "{ui} is a folder: it is not replaced by a file"
            )));
        }
    }
    let tmp = crate::sftp::free_local_path(
        dest.parent().unwrap_or(Path::new(".")),
        &format!(".{name}.part"),
    );
    Ok((dest, tmp, ui))
}

/// Move the finished temporary file over the destination (replacing it), or remove it on error.
/// A replaced file keeps its permissions.
pub(crate) fn finish(tmp: &Path, dest: &Path, done: Result<u64>) -> Result<()> {
    let done = done.map(|n| {
        if let Ok(m) = std::fs::metadata(dest) {
            let _ = std::fs::set_permissions(tmp, m.permissions());
        }
        n
    });
    match done.and_then(|_| {
        std::fs::rename(tmp, dest).map_err(|e| Error::Other(format!("{}: {e}", dest.display())))
    }) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(tmp);
            Err(e)
        }
    }
}

/// Copy a local file to another local folder (through a temporary file).
pub(crate) async fn copy_local(
    src: &Path,
    tmp: &Path,
    mut progress: impl FnMut(u64, u64),
) -> Result<u64> {
    let total = std::fs::metadata(src).map(|m| m.len()).unwrap_or(0);
    progress(0, total);
    let n = tokio::fs::copy(src, tmp)
        .await
        .map_err(|e| Error::Other(format!("{}: {e}", src.display())))?;
    progress(n, n);
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_paths() {
        assert_eq!(
            native_in("/home/me", false).unwrap().unwrap(),
            PathBuf::from("/home/me")
        );
        assert_eq!(native_in("/", false).unwrap().unwrap(), PathBuf::from("/"));
        assert_eq!(
            native_in("/a//b/./c/", false).unwrap().unwrap(),
            PathBuf::from("/a/b/c")
        );
        assert!(native_in("home", false).is_err());
        assert!(native_in("/a/../etc", false).is_err());
        assert_eq!(ui_in("/home/me", false), "/home/me");
    }

    #[test]
    fn windows_paths() {
        assert_eq!(native_in("/", true).unwrap(), None);
        assert_eq!(
            native_in("/C:", true).unwrap().unwrap(),
            PathBuf::from("C:\\")
        );
        assert_eq!(
            native_in("/c:/Users/me", true).unwrap().unwrap(),
            PathBuf::from("C:\\Users\\me")
        );
        assert!(native_in("/Users", true).is_err());
        assert!(native_in("/C:/a/../b", true).is_err());
        assert!(native_in("/C:/a\\..\\b", true).is_err());
        assert_eq!(ui_in("C:\\Users\\me", true), "/C:/Users/me");
        assert_eq!(ui_in("C:\\", true), "/C:");
        // Round trip.
        let p = native_in(&ui_in("D:\\work\\x.txt", true), true)
            .unwrap()
            .unwrap();
        assert_eq!(p, PathBuf::from("D:\\work\\x.txt"));
    }

    #[test]
    fn joins() {
        assert_eq!(join_ui("/", "etc"), "/etc");
        assert_eq!(join_ui("/home/me/", "a"), "/home/me/a");
        assert_eq!(join_ui("/C:", "Users"), "/C:/Users");
    }
}
