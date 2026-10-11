//! SFTP over an already authenticated SSH connection: the `sftp` subsystem on a new channel of
//! the same connection, so a terminal's file browser needs no second login. The protocol itself
//! is `russh-sftp`'s client, which runs over any byte stream.

use crate::client::{net, SshConnection};
use aht_core::{Error, Result};
use russh::ChannelMsg;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::{FileAttributes, FileType, StatusCode};
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
    /// For a symlink, the kind of its target (`Symlink` when the target cannot be read).
    pub kind: EntryKind,
    pub link: bool,
    pub size: u64,
    /// Seconds since the Unix epoch.
    pub modified: Option<u32>,
    /// Unix mode bits (type and permissions), when the server reports them.
    pub permissions: Option<u32>,
}

/// What a file looked like when read, to notice someone else changing it before a save.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStamp {
    pub size: u64,
    /// Seconds since the Unix epoch.
    pub modified: Option<u32>,
    /// Unix mode bits, when the server reports them.
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

/// `base/rel`, or `base` itself for the empty `rel`.
fn join_rel(base: &str, rel: &str) -> String {
    if rel.is_empty() {
        base.to_string()
    } else {
        join(base, rel)
    }
}

/// Error text of a copy whose destination already exists (and `overwrite` was not given).
pub const EXISTS: &str = "exists";

/// One step of a copy: a folder to create or a file to copy, relative to the copied item.
struct CopyItem {
    rel: String,
    dir: bool,
    size: u64,
    mode: Option<u32>,
}

/// Printed on stdout once sudo let the command run, before `sftp-server` starts.
const SUDO_OK: &str = "HANDRAIL-SUDO-OK";
const NO_SERVER: &str = "HANDRAIL-NO-SFTP-SERVER";
/// Error text that means "ask the user for the sudo password".
pub const SUDO_PASSWORD_REQUIRED: &str = "sudo_password_required";

/// The shell command that runs the first `sftp-server` found as root.
fn sudo_command(with_password: bool) -> String {
    let script = format!(
        "for p in /usr/lib/openssh/sftp-server /usr/libexec/openssh/sftp-server \
         /usr/lib/ssh/sftp-server /usr/libexec/sftp-server /usr/lib/sftp-server \
         /usr/local/libexec/sftp-server; do if [ -x \"$p\" ]; then echo {SUDO_OK}; \
         exec \"$p\"; fi; done; echo {NO_SERVER}; exit 127"
    );
    // -n never prompts; -S reads the password from stdin (exactly one line); -p '' keeps the
    // prompt off the stream.
    let mode = if with_password { "-S -p ''" } else { "-n" };
    format!("sudo {mode} sh -c '{script}'")
}

/// The error sudo's stderr stands for.
fn sudo_error(stderr: &str, with_password: bool) -> Error {
    let s = stderr.to_lowercase();
    if s.contains("password is required") || (!with_password && s.contains("password")) {
        Error::Auth(SUDO_PASSWORD_REQUIRED.into())
    } else if s.contains("sorry, try again") || s.contains("incorrect password") {
        Error::Auth("wrong sudo password".into())
    } else if s.contains("tty") {
        Error::Denied("sudo on this server requires a terminal (requiretty)".into())
    } else if s.contains("not in the sudoers") || s.contains("not allowed") {
        Error::Denied(format!("sudo refused: {}", stderr.trim()))
    } else if s.trim().is_empty() {
        Error::Other("sudo ended without starting the SFTP server".into())
    } else {
        Error::Other(format!("sudo: {}", stderr.trim()))
    }
}

impl SshConnection {
    /// Start an SFTP server as root through `sudo` on a new channel (what "browse as root" uses;
    /// becoming root in the terminal cannot change the user of an SFTP channel). Without a
    /// password it uses `sudo -n` and fails with [`SUDO_PASSWORD_REQUIRED`] when one is needed.
    pub async fn open_sftp_sudo(&self, password: Option<&str>) -> Result<Sftp> {
        let with_password = password.is_some();
        let mut ch = self.handle.channel_open_session().await.map_err(net)?;
        ch.exec(true, sudo_command(with_password))
            .await
            .map_err(net)?;
        if let Some(pw) = password {
            let line = zeroize::Zeroizing::new(format!("{pw}\n"));
            ch.data(line.as_bytes()).await.map_err(net)?;
        }
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let started = tokio::time::timeout(std::time::Duration::from_secs(20), async {
            loop {
                match ch.wait().await {
                    Some(ChannelMsg::Data { data }) => {
                        stdout.extend_from_slice(&data);
                        let out = String::from_utf8_lossy(&stdout);
                        if out.contains(NO_SERVER) {
                            return Err(Error::NotFound(
                                "no sftp-server program found on the server".into(),
                            ));
                        }
                        if let Some(i) = out.find(SUDO_OK) {
                            // sftp-server says nothing before our init, so nothing may follow.
                            if out[i + SUDO_OK.len()..].trim().is_empty() {
                                return Ok(());
                            }
                            return Err(Error::Other("unexpected output from sudo".into()));
                        }
                    }
                    Some(ChannelMsg::ExtendedData { data, .. }) => {
                        stderr.extend_from_slice(&data);
                        // A wrong password makes sudo wait for another line: stop here.
                        let err = String::from_utf8_lossy(&stderr).to_lowercase();
                        if err.contains("sorry, try again") || err.contains("incorrect password") {
                            return Err(sudo_error(&err, with_password));
                        }
                    }
                    Some(ChannelMsg::Failure) => {
                        return Err(Error::Denied("the server refused to run sudo".into()))
                    }
                    Some(ChannelMsg::ExitStatus { .. })
                    | Some(ChannelMsg::Eof)
                    | Some(ChannelMsg::Close)
                    | None => {
                        return Err(sudo_error(&String::from_utf8_lossy(&stderr), with_password))
                    }
                    Some(_) => {}
                }
            }
        })
        .await
        .unwrap_or_else(|_| Err(Error::Other("sudo did not answer in time".into())));
        if let Err(e) = started {
            let _ = ch.close().await;
            return Err(e);
        }
        let session = SftpSession::new(ch.into_stream()).await.map_err(sftp_err)?;
        Ok(Sftp { session })
    }

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

    /// Entries of `dir` without `.` and `..`: folders first, then by name. Symlinks are followed
    /// to tell links to folders from links to files.
    pub async fn list(&self, dir: &str) -> Result<Vec<RemoteEntry>> {
        let kind_of = |t: FileType| match t {
            FileType::Dir => EntryKind::Dir,
            FileType::File => EntryKind::File,
            FileType::Symlink => EntryKind::Symlink,
            FileType::Other => EntryKind::Other,
        };
        let mut out = Vec::new();
        for e in self.session.read_dir(dir).await.map_err(sftp_err)? {
            let name = e.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let path = join(dir, &name);
            let m = e.metadata();
            let link = e.file_type() == FileType::Symlink;
            // `metadata` follows the link; a dangling one stays a plain symlink.
            let target = if link {
                self.session.metadata(&path).await.ok()
            } else {
                None
            };
            out.push(RemoteEntry {
                kind: kind_of(target.as_ref().map_or(e.file_type(), |t| t.file_type())),
                size: target.as_ref().unwrap_or(&m).size.unwrap_or(0),
                link,
                name,
                path,
                modified: m.mtime,
                permissions: m.permissions,
            });
        }
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

    /// Size and modification time of `path` (following links).
    pub async fn stamp(&self, path: &str) -> Result<FileStamp> {
        let m = self.session.metadata(path).await.map_err(sftp_err)?;
        Ok(FileStamp {
            size: m.size.unwrap_or(0),
            modified: m.mtime,
            permissions: m.permissions,
        })
    }

    /// The whole file, refusing files over `limit` bytes.
    pub async fn read_all(&self, path: &str, limit: u64) -> Result<Vec<u8>> {
        let mut f = self.session.open(path).await.map_err(sftp_err)?;
        let size = f.metadata().await.map_err(sftp_err)?.size.unwrap_or(0);
        if size > limit {
            return Err(Error::Invalid(format!(
                "{path} is {size} bytes; files over {limit} bytes are not opened"
            )));
        }
        let mut out = Vec::with_capacity(size as usize);
        let mut buf = vec![0u8; CHUNK];
        let read = async {
            loop {
                let n = f.read(&mut buf).await?;
                if n == 0 {
                    return Ok(());
                }
                out.extend_from_slice(&buf[..n]);
                if out.len() as u64 > limit {
                    return Err(std::io::Error::other(format!(
                        "{path} grew past {limit} bytes"
                    )));
                }
            }
        }
        .await;
        let _ = f.close().await;
        read.map_err(io_err)?;
        Ok(out)
    }

    /// Replace the contents of `path` in place (created if missing). Writing into the existing
    /// file keeps its owner and permissions, unlike writing a copy and renaming it.
    pub async fn write_all(&self, path: &str, data: &[u8]) -> Result<()> {
        self.write_all_mode(path, data, None).await
    }

    /// [`Self::write_all`], first giving the file the permission bits of `mode` (before any byte
    /// is written), e.g. so a backup of a private file is not created world-readable.
    pub async fn write_all_mode(&self, path: &str, data: &[u8], mode: Option<u32>) -> Result<()> {
        let mut f = self.session.create(path).await.map_err(sftp_err)?;
        if let Some(mode) = mode {
            let attrs = FileAttributes {
                permissions: Some(mode & 0o7777),
                ..FileAttributes::empty()
            };
            f.set_metadata(attrs).await.map_err(sftp_err)?;
        }
        for chunk in data.chunks(CHUNK) {
            f.write_all(chunk).await.map_err(io_err)?;
        }
        f.close().await.map_err(io_err)?;
        Ok(())
    }

    /// What copying `path` involves: the item itself and, for a folder, everything under it.
    /// Links to folders are not followed (they can loop); links to files are copied as files;
    /// dangling links and special files are left out.
    async fn copy_plan(&self, path: &str) -> Result<Vec<CopyItem>> {
        let m = self.session.metadata(path).await.map_err(sftp_err)?;
        if m.file_type() != FileType::Dir {
            return Ok(vec![CopyItem {
                rel: String::new(),
                dir: false,
                size: m.size.unwrap_or(0),
                mode: m.permissions,
            }]);
        }
        let mut items = vec![CopyItem {
            rel: String::new(),
            dir: true,
            size: 0,
            mode: m.permissions,
        }];
        let mut todo = vec![String::new()];
        while let Some(rel) = todo.pop() {
            for e in self.list(&join_rel(path, &rel)).await? {
                let r = join_rel(&rel, &e.name);
                match e.kind {
                    EntryKind::Dir if !e.link => {
                        items.push(CopyItem {
                            rel: r.clone(),
                            dir: true,
                            size: 0,
                            mode: e.permissions,
                        });
                        todo.push(r);
                    }
                    EntryKind::File => items.push(CopyItem {
                        rel: r,
                        dir: false,
                        size: e.size,
                        // A link's own mode is not its target's.
                        mode: if e.link { None } else { e.permissions },
                    }),
                    _ => {}
                }
            }
        }
        Ok(items)
    }

    /// Copy `from` (a file, or a folder with everything in it) to `to` on `dest`, the SFTP of
    /// another connection (or this one). The bytes stream through this computer without
    /// touching its disk; permission bits are kept. Fails with `Busy(EXISTS)` when `to` exists,
    /// unless `overwrite` (files are replaced, folders merged). `progress(done, total)` follows
    /// the bytes of all files.
    pub async fn copy_to(
        &self,
        from: &str,
        dest: &Sftp,
        to: &str,
        overwrite: bool,
        mut progress: impl FnMut(u64, u64) + Send,
    ) -> Result<u64> {
        if !overwrite && dest.session.metadata(to).await.is_ok() {
            return Err(Error::Busy(EXISTS.into()));
        }
        let items = self.copy_plan(from).await?;
        let total = items.iter().map(|i| i.size).sum();
        let mut done = 0u64;
        progress(0, total);
        let mut buf = vec![0u8; CHUNK];
        for it in &items {
            let (src, dst) = (join_rel(from, &it.rel), join_rel(to, &it.rel));
            let attrs = it.mode.map(|m| FileAttributes {
                permissions: Some(m & 0o7777),
                ..FileAttributes::empty()
            });
            if it.dir {
                if let Err(e) = dest.session.create_dir(&dst).await {
                    if !dest.is_dir(&dst).await.unwrap_or(false) {
                        return Err(sftp_err(e));
                    }
                }
                if let Some(a) = attrs {
                    let _ = dest.session.set_metadata(&dst, a).await;
                }
                continue;
            }
            let mut r = self.session.open(&src).await.map_err(sftp_err)?;
            let mut w = dest.session.create(&dst).await.map_err(sftp_err)?;
            if let Some(a) = attrs {
                let _ = w.set_metadata(a).await;
            }
            let copied = async {
                loop {
                    let n = r.read(&mut buf).await?;
                    if n == 0 {
                        return Ok::<(), std::io::Error>(());
                    }
                    w.write_all(&buf[..n]).await?;
                    done += n as u64;
                    progress(done, total);
                }
            }
            .await;
            let _ = r.close().await;
            copied.map_err(io_err)?;
            w.close().await.map_err(io_err)?;
        }
        progress(done, total);
        Ok(done)
    }

    pub async fn close(&self) {
        let _ = self.session.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sudo_commands_and_errors() {
        assert!(sudo_command(false).starts_with("sudo -n sh -c '"));
        assert!(sudo_command(true).starts_with("sudo -S -p '' sh -c '"));
        assert!(!sudo_command(true).contains('\n'));
        let need = sudo_error("sudo: a password is required\n", false);
        assert!(matches!(need, Error::Auth(m) if m == SUDO_PASSWORD_REQUIRED));
        let wrong = sudo_error("Sorry, try again.\n", true);
        assert!(matches!(wrong, Error::Auth(m) if m == "wrong sudo password"));
        assert!(matches!(
            sudo_error("sudo: sorry, you must have a tty to run sudo", false),
            Error::Denied(_)
        ));
        assert!(matches!(
            sudo_error("bob is not in the sudoers file.", true),
            Error::Denied(_)
        ));
    }

    #[test]
    fn joins_relative_paths() {
        assert_eq!(join_rel("/srv/app", ""), "/srv/app");
        assert_eq!(join_rel("/srv/app", "a/b.txt"), "/srv/app/a/b.txt");
    }

    #[test]
    fn joins_posix_paths() {
        assert_eq!(join("/home/me", "a.txt"), "/home/me/a.txt");
        assert_eq!(join("/", "etc"), "/etc");
        assert_eq!(join("", "x"), "x");
    }
}
