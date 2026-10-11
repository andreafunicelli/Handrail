//! SFTP over an already authenticated SSH connection: the `sftp` subsystem on a new channel of
//! the same connection, so a terminal's file browser needs no second login. The protocol itself
//! is `russh-sftp`'s client, which runs over any byte stream.

use crate::client::{net, SshConnection};
use aht_core::{Error, Result};
use russh::ChannelMsg;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::{FileType, StatusCode};
use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Bytes moved per read/write call during transfers.
const CHUNK: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Dir,
    File,
    Symlink,
    Other,
}

/// One entry of a remote directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteEntry {
    pub name: String,
    pub path: String,
    pub kind: EntryKind,
    pub size: u64,
    /// Seconds since the Unix epoch.
    pub modified: Option<u32>,
    /// Unix mode bits (type and permissions), when the server reports them.
    pub permissions: Option<u32>,
}

/// An SFTP session on one SSH connection.
pub struct Sftp {
    session: SftpSession,
}

impl std::fmt::Debug for Sftp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Sftp")
    }
}

fn sftp_err(e: SftpError) -> Error {
    match &e {
        SftpError::Status(s) => match s.status_code {
            StatusCode::NoSuchFile => Error::NotFound(s.error_message.clone()),
            StatusCode::PermissionDenied => Error::Denied(s.error_message.clone()),
            _ => Error::Other(format!("SFTP: {}", s.error_message)),
        },
        _ => Error::Other(format!("SFTP: {e}")),
    }
}

fn io_err(e: std::io::Error) -> Error {
    Error::Other(format!("SFTP transfer: {e}"))
}

/// `dir` joined with `name` using `/` (SFTP paths are POSIX on the wire).
pub fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

impl SshConnection {
    /// Start the `sftp` subsystem on a new channel of this connection.
    pub async fn open_sftp(&self) -> Result<Sftp> {
        let mut ch = self.handle.channel_open_session().await.map_err(net)?;
        ch.request_subsystem(true, "sftp").await.map_err(net)?;
        loop {
            match ch.wait().await {
                Some(ChannelMsg::Success) => break,
                Some(ChannelMsg::Failure) => {
                    return Err(Error::Denied(
                        "the server does not offer SFTP (sftp subsystem refused)".into(),
                    ))
                }
                Some(ChannelMsg::Close) | None => {
                    return Err(net("channel closed while starting SFTP"))
                }
                Some(_) => {}
            }
        }
        let session = SftpSession::new(ch.into_stream()).await.map_err(sftp_err)?;
        Ok(Sftp { session })
    }
}

impl Sftp {
    /// Absolute form of `path` (`.` is the login directory).
    pub async fn canonicalize(&self, path: &str) -> Result<String> {
        self.session.canonicalize(path).await.map_err(sftp_err)
    }

    /// Entries of `dir` without `.` and `..`: folders first, then by name.
    pub async fn list(&self, dir: &str) -> Result<Vec<RemoteEntry>> {
        let mut out: Vec<RemoteEntry> = self
            .session
            .read_dir(dir)
            .await
            .map_err(sftp_err)?
            .filter(|e| e.file_name() != "." && e.file_name() != "..")
            .map(|e| {
                let m = e.metadata();
                RemoteEntry {
                    name: e.file_name(),
                    path: join(dir, &e.file_name()),
                    kind: match e.file_type() {
                        FileType::Dir => EntryKind::Dir,
                        FileType::File => EntryKind::File,
                        FileType::Symlink => EntryKind::Symlink,
                        FileType::Other => EntryKind::Other,
                    },
                    size: m.size.unwrap_or(0),
                    modified: m.mtime,
                    permissions: m.permissions,
                }
            })
            .collect();
        out.sort_by(|a, b| {
            (b.kind == EntryKind::Dir)
                .cmp(&(a.kind == EntryKind::Dir))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(out)
    }

    /// Whether `path` is a directory (following symlinks).
    pub async fn is_dir(&self, path: &str) -> Result<bool> {
        let m = self.session.metadata(path).await.map_err(sftp_err)?;
        Ok(m.file_type() == FileType::Dir)
    }

    pub async fn mkdir(&self, path: &str) -> Result<()> {
        self.session.create_dir(path).await.map_err(sftp_err)
    }

    pub async fn rename(&self, from: &str, to: &str) -> Result<()> {
        self.session.rename(from, to).await.map_err(sftp_err)
    }

    /// Delete a file, or an empty directory.
    pub async fn remove(&self, path: &str, is_dir: bool) -> Result<()> {
        if is_dir {
            self.session.remove_dir(path).await.map_err(sftp_err)
        } else {
            self.session.remove_file(path).await.map_err(sftp_err)
        }
    }

    /// Copy `remote` to the new local file `local`. `progress(done, total)` follows the bytes.
    /// A failed download removes the partial local file.
    pub async fn download(
        &self,
        remote: &str,
        local: &Path,
        mut progress: impl FnMut(u64, u64) + Send,
    ) -> Result<u64> {
        let mut src = self.session.open(remote).await.map_err(sftp_err)?;
        let total = src.metadata().await.map_err(sftp_err)?.size.unwrap_or(0);
        let mut dst = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(local)
            .await
            .map_err(io_err)?;
        let copied = async {
            let mut buf = vec![0u8; CHUNK];
            let mut done = 0u64;
            progress(0, total);
            loop {
                let n = src.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                dst.write_all(&buf[..n]).await?;
                done += n as u64;
                progress(done, total);
            }
            dst.flush().await?;
            Ok::<u64, std::io::Error>(done)
        }
        .await;
        let _ = src.close().await;
        match copied {
            Ok(done) => Ok(done),
            Err(e) => {
                drop(dst);
                let _ = tokio::fs::remove_file(local).await;
                Err(io_err(e))
            }
        }
    }

    /// Copy the local file `local` to `remote` (created or replaced).
    pub async fn upload(
        &self,
        local: &Path,
        remote: &str,
        mut progress: impl FnMut(u64, u64) + Send,
    ) -> Result<u64> {
        let mut src = tokio::fs::File::open(local).await.map_err(io_err)?;
        let total = src.metadata().await.map_err(io_err)?.len();
        let mut dst = self.session.create(remote).await.map_err(sftp_err)?;
        let mut buf = vec![0u8; CHUNK];
        let mut done = 0u64;
        progress(0, total);
        loop {
            let n = src.read(&mut buf).await.map_err(io_err)?;
            if n == 0 {
                break;
            }
            dst.write_all(&buf[..n]).await.map_err(io_err)?;
            done += n as u64;
            progress(done, total);
        }
        dst.close().await.map_err(io_err)?;
        Ok(done)
    }

    pub async fn close(&self) {
        let _ = self.session.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::join;

    #[test]
    fn joins_posix_paths() {
        assert_eq!(join("/home/me", "a.txt"), "/home/me/a.txt");
        assert_eq!(join("/", "etc"), "/etc");
        assert_eq!(join("", "x"), "x");
    }
}
