//! Local shells via portable-pty (ConPTY on Windows), exposed through the same `Transport`.

use aht_core::terminal::{Transport, TransportEvent};
use aht_core::{Error, Result};
use async_trait::async_trait;
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalShellSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
    /// Variables removed from the inherited environment (e.g. API keys a harness must not see).
    pub env_remove: Vec<String>,
    pub cols: u16,
    pub rows: u16,
}

impl LocalShellSpec {
    pub fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
            cwd: None,
            env: Vec::new(),
            env_remove: Vec::new(),
            cols: 80,
            rows: 24,
        }
    }
}

fn pty_err(e: impl std::fmt::Display) -> Error {
    Error::Io(format!("pty: {e}"))
}

fn size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

type Shared<T> = Arc<Mutex<Option<T>>>;

/// A running local PTY process.
pub struct LocalPty {
    master: Shared<Box<dyn MasterPty + Send>>,
    writer: Shared<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
}

impl std::fmt::Debug for LocalPty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LocalPty")
    }
}

/// Spawn `spec` in a new PTY. Output (and finally `Closed { exit_status }`) arrives on the receiver.
pub fn spawn_local(spec: LocalShellSpec) -> Result<(LocalPty, mpsc::Receiver<TransportEvent>)> {
    let pair = native_pty_system()
        .openpty(size(spec.cols, spec.rows))
        .map_err(pty_err)?;
    let mut cmd = CommandBuilder::new(&spec.program);
    cmd.args(&spec.args);
    if let Some(cwd) = &spec.cwd {
        cmd.cwd(cwd);
    }
    if !spec.env.iter().any(|(k, _)| k == "TERM") {
        cmd.env("TERM", "xterm-256color");
    }
    for k in &spec.env_remove {
        cmd.env_remove(k);
    }
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    let mut child = pair.slave.spawn_command(cmd).map_err(pty_err)?;
    // Drop our copy of the slave so EOF is seen when the child exits (Unix).
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().map_err(pty_err)?;
    let writer = pair.master.take_writer().map_err(pty_err)?;
    let killer = child.clone_killer();
    let master: Shared<Box<dyn MasterPty + Send>> = Arc::new(Mutex::new(Some(pair.master)));
    let writer: Shared<Box<dyn Write + Send>> = Arc::new(Mutex::new(Some(writer)));

    let (tx, rx) = mpsc::channel(256);
    let (exit_tx, exit_rx) = std::sync::mpsc::channel::<Option<u32>>();

    // Waiter: on exit, close the PTY so the (blocking) reader sees EOF — ConPTY needs this.
    let (m2, w2) = (master.clone(), writer.clone());
    std::thread::Builder::new()
        .name("aht-pty-wait".into())
        .spawn(move || {
            let code = child.wait().ok().map(|s| s.exit_code());
            let _ = exit_tx.send(code);
            // Give the reader a moment to drain buffered output before tearing down the PTY.
            std::thread::sleep(std::time::Duration::from_millis(100));
            w2.lock().unwrap_or_else(|e| e.into_inner()).take();
            m2.lock().unwrap_or_else(|e| e.into_inner()).take();
        })
        .map_err(pty_err)?;

    std::thread::Builder::new()
        .name("aht-pty-read".into())
        .spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx
                            .blocking_send(TransportEvent::Data(buf[..n].to_vec()))
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            }
            let code = exit_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .ok()
                .flatten();
            let _ = tx.blocking_send(TransportEvent::Closed { exit_status: code });
        })
        .map_err(pty_err)?;

    Ok((
        LocalPty {
            master,
            writer,
            killer: Mutex::new(killer),
        },
        rx,
    ))
}

#[async_trait]
impl Transport for LocalPty {
    async fn write(&self, data: &[u8]) -> Result<()> {
        let mut g = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        let w = g.as_mut().ok_or_else(|| Error::Io("pty closed".into()))?;
        w.write_all(data)?;
        w.flush()?;
        Ok(())
    }

    async fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        let g = self.master.lock().unwrap_or_else(|e| e.into_inner());
        let m = g.as_ref().ok_or_else(|| Error::Io("pty closed".into()))?;
        m.resize(size(cols, rows)).map_err(pty_err)
    }

    async fn close(&self) -> Result<()> {
        let _ = self.killer.lock().unwrap_or_else(|e| e.into_inner()).kill();
        Ok(())
    }
}

/// A shell found on this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredShell {
    /// Stable id, e.g. "pwsh", "cmd", "git-bash", "wsl:Ubuntu", "zsh".
    pub id: String,
    pub label: String,
    pub program: String,
    pub args: Vec<String>,
}

impl DiscoveredShell {
    pub fn to_spec(&self, cols: u16, rows: u16) -> LocalShellSpec {
        LocalShellSpec {
            cols,
            rows,
            ..LocalShellSpec::new(self.program.clone(), self.args.clone())
        }
    }
}

/// Find `name` on PATH (adds `.exe` on Windows when no extension given).
pub fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let cand = dir.join(name);
        if cand.is_file() {
            return Some(cand);
        }
        if cfg!(windows) && cand.extension().is_none() {
            let exe = cand.with_extension("exe");
            if exe.is_file() {
                return Some(exe);
            }
        }
    }
    None
}

/// Parse the UTF-16LE output of `wsl.exe -l -q` into distro names.
pub fn parse_wsl_list(raw: &[u8]) -> Vec<String> {
    let text = if raw.len() >= 2 && raw.iter().skip(1).step_by(2).all(|b| *b == 0) {
        let units: Vec<u16> = raw
            .chunks(2)
            .map(|c| u16::from_le_bytes([c[0], c.get(1).copied().unwrap_or(0)]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(raw).into_owned()
    };
    text.lines()
        .map(|l| l.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}' || c == '\0'))
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

fn shell(id: &str, label: &str, program: PathBuf, args: &[&str]) -> DiscoveredShell {
    DiscoveredShell {
        id: id.into(),
        label: label.into(),
        program: program.to_string_lossy().into_owned(),
        args: args.iter().map(|s| s.to_string()).collect(),
    }
}

/// Discover local shells. Windows: pwsh, powershell, cmd, Git Bash, WSL distros.
/// Unix: `$SHELL`, bash, zsh, fish.
pub fn default_local_shells() -> Vec<DiscoveredShell> {
    let mut out = Vec::new();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if let Some(p) = find_in_path("pwsh") {
            out.push(shell("pwsh", "PowerShell", p, &["-NoLogo"]));
        }
        if let Some(p) = find_in_path("powershell") {
            out.push(shell("powershell", "Windows PowerShell", p, &["-NoLogo"]));
        }
        let cmd = std::env::var_os("COMSPEC")
            .map(PathBuf::from)
            .or_else(|| find_in_path("cmd"));
        if let Some(p) = cmd {
            out.push(shell("cmd", "Command Prompt", p, &[]));
        }
        let mut git_bash = Vec::new();
        if let Some(git) = find_in_path("git") {
            // <Git>\cmd\git.exe -> <Git>\bin\bash.exe
            if let Some(root) = git.parent().and_then(|p| p.parent()) {
                git_bash.push(root.join("bin").join("bash.exe"));
            }
        }
        for var in ["ProgramFiles", "ProgramW6432", "LOCALAPPDATA"] {
            if let Some(base) = std::env::var_os(var) {
                let base = PathBuf::from(base);
                git_bash.push(base.join("Git").join("bin").join("bash.exe"));
                git_bash.push(
                    base.join("Programs")
                        .join("Git")
                        .join("bin")
                        .join("bash.exe"),
                );
            }
        }
        if let Some(p) = git_bash.into_iter().find(|p| p.is_file()) {
            out.push(shell("git-bash", "Git Bash", p, &["--login", "-i"]));
        }
        if let Some(wsl) = find_in_path("wsl") {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            if let Ok(o) = std::process::Command::new(&wsl)
                .args(["-l", "-q"])
                .creation_flags(CREATE_NO_WINDOW)
                .output()
            {
                if o.status.success() {
                    for d in parse_wsl_list(&o.stdout) {
                        out.push(shell(
                            &format!("wsl:{d}"),
                            &format!("WSL: {d}"),
                            wsl.clone(),
                            &["-d", &d],
                        ));
                    }
                }
            }
        }
    }
    #[cfg(not(windows))]
    {
        let mut seen = std::collections::HashSet::new();
        let login = std::env::var_os("SHELL").map(PathBuf::from);
        let candidates = login.into_iter().filter(|p| p.is_file()).chain(
            ["bash", "zsh", "fish"]
                .iter()
                .filter_map(|n| find_in_path(n)),
        );
        for p in candidates {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if seen.insert(name.clone()) {
                out.push(shell(&name, &name, p, &["-l"]));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wsl_utf16() {
        let s = "Ubuntu\r\ndocker-desktop\r\n\r\n";
        let mut raw = vec![0xff, 0xfe];
        raw.clear();
        for u in s.encode_utf16() {
            raw.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(parse_wsl_list(&raw), vec!["Ubuntu", "docker-desktop"]);
        assert_eq!(parse_wsl_list(b"Debian\n"), vec!["Debian"]);
    }

    #[test]
    fn discovery_finds_something() {
        let shells = default_local_shells();
        assert!(!shells.is_empty(), "no shells found");
        for s in &shells {
            assert!(!s.program.is_empty());
        }
    }

    #[tokio::test]
    async fn local_pty_echo() {
        let spec = if cfg!(windows) {
            LocalShellSpec::new("cmd", vec!["/c".into(), "echo hi".into()])
        } else {
            LocalShellSpec::new("sh", vec!["-c".into(), "echo hi".into()])
        };
        let (pty, mut rx) = spawn_local(spec).unwrap();
        pty.resize(100, 30).await.unwrap_or(()); // may race with exit
        let mut out = Vec::new();
        let closed = tokio::time::timeout(std::time::Duration::from_secs(15), async {
            while let Some(ev) = rx.recv().await {
                match ev {
                    TransportEvent::Data(d) => {
                        // ConPTY asks for the cursor position (DSR) and blocks until a terminal
                        // answers; the real terminal emulator does this in the app.
                        if d.windows(4).any(|w| w == b"\x1b[6n") {
                            pty.write(b"\x1b[1;1R").await.unwrap();
                        }
                        out.extend(d)
                    }
                    TransportEvent::Closed { exit_status } => return exit_status,
                    TransportEvent::Error(e) => panic!("{e}"),
                }
            }
            panic!("channel ended without Closed");
        })
        .await
        .expect("timed out");
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("hi"), "output: {text:?}");
        assert_eq!(closed, Some(0));
    }
}
