//! Validates the injected shell hooks against real shells spawned in a PTY (ConPTY on Windows).
//! Skips (passes with a message) when the shell is not on PATH.

mod common;

use aht_core::model::{ApprovalMode, Environment};
use aht_core::terminal::{
    Actor, CommandState, IntegrationMode, RunCommandRequest, SessionState, Transport,
    TransportEvent,
};
use aht_core::{Error, Result};
use aht_term::{Policy, TermConfig, TermSession};
use async_trait::async_trait;
use common::TestApprover;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

struct PtyTransport {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    child: Mutex<Box<dyn portable_pty::Child + Send + Sync>>,
}

#[async_trait]
impl Transport for PtyTransport {
    async fn write(&self, data: &[u8]) -> Result<()> {
        let mut w = self.writer.lock().unwrap();
        w.write_all(data)?;
        w.flush()?;
        Ok(())
    }
    async fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.master
            .lock()
            .unwrap()
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| Error::Io(e.to_string()))
    }
    async fn close(&self) -> Result<()> {
        let _ = self.child.lock().unwrap().kill();
        Ok(())
    }
}

fn on_path(prog: &str) -> bool {
    std::process::Command::new(prog)
        .arg(if prog.contains("pwsh") {
            "-Version"
        } else {
            "--version"
        })
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn spawn_pty(prog: &str, args: &[&str]) -> (Arc<PtyTransport>, mpsc::Receiver<TransportEvent>) {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 40,
            cols: 250,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut cmd = CommandBuilder::new(prog);
    cmd.args(args);
    cmd.env("TERM", "xterm-256color");
    let child = pair.slave.spawn_command(cmd).unwrap();
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let writer: Arc<Mutex<Box<dyn Write + Send>>> =
        Arc::new(Mutex::new(pair.master.take_writer().unwrap()));
    let (tx, rx) = mpsc::channel(4096);
    let w2 = writer.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => {
                    let _ = tx.blocking_send(TransportEvent::Closed { exit_status: None });
                    break;
                }
                Ok(n) => {
                    if std::env::var("AHT_DUMP").is_ok() {
                        eprintln!("RAW {:?}", String::from_utf8_lossy(&buf[..n]));
                    }
                    // ConPTY asks for the cursor position at startup and blocks until answered;
                    // in the app xterm.js answers it.
                    if buf[..n].windows(4).any(|x| x == b"\x1b[6n") {
                        let _ = w2.lock().unwrap().write_all(b"\x1b[1;1R");
                    }
                    if tx
                        .blocking_send(TransportEvent::Data(buf[..n].to_vec()))
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    });
    (
        Arc::new(PtyTransport {
            writer,
            master: Mutex::new(pair.master),
            child: Mutex::new(child),
        }),
        rx,
    )
}

fn actor() -> Actor {
    Actor {
        id: "test".into(),
        label: "Test".into(),
        session_scope: None,
    }
}

fn req(cmd: &str) -> RunCommandRequest {
    RunCommandRequest {
        session_id: aht_core::SessionId::new(),
        command: cmd.into(),
        timeout_s: Some(30),
        reason: None,
    }
}

async fn start(prog: &str, args: &[&str]) -> (TermSession, Arc<Mutex<Vec<u8>>>) {
    let (t, rx) = spawn_pty(prog, args);
    let s = TermSession::spawn(
        t,
        rx,
        TermConfig {
            policy: Policy {
                mode: ApprovalMode::Auto,
                env: Environment::Dev,
                deny_list: vec![],
            },
            detect_timeout: Duration::from_secs(10),
            probe_quiescence: Duration::from_millis(800),
            ..Default::default()
        },
    );
    let ui = Arc::new(Mutex::new(Vec::new()));
    let mut rx = s.subscribe_ui();
    let ui2 = ui.clone();
    tokio::spawn(async move {
        while let Ok(c) = rx.recv().await {
            ui2.lock().unwrap().extend(c);
        }
    });
    for _ in 0..600 {
        if s.integration() != IntegrationMode::Unknown && s.state() == SessionState::Idle {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    (s, ui)
}

#[tokio::test(flavor = "multi_thread")]
async fn real_bash_hook_marks_and_exit_codes() {
    if !on_path("bash") {
        eprintln!("bash not on PATH; skipping");
        return;
    }
    let (s, ui) = start("bash", &["--norc", "-i"]).await;
    assert_eq!(
        s.integration(),
        IntegrationMode::Full,
        "hook did not install"
    );
    let info = s.info();
    assert_eq!(info.shell.as_deref(), Some("bash"));
    assert!(
        info.os.is_some() && info.user.is_some() && info.host.is_some(),
        "{info:?}"
    );
    assert!(info.cwd.is_some(), "OSC 7 cwd: {info:?}");

    let ap = TestApprover::approve();
    let r = s
        .run_command(&actor(), req("echo hello"), &ap)
        .await
        .unwrap();
    assert_eq!(r.state, CommandState::Completed, "{r:?}");
    assert_eq!(r.exit_code, Some(0));
    assert_eq!(r.output.trim(), "hello");

    let r = s.run_command(&actor(), req("false"), &ap).await.unwrap();
    assert_eq!(r.exit_code, Some(1), "{r:?}");
    let r = s.run_command(&actor(), req("(exit 3)"), &ap).await.unwrap();
    assert_eq!(r.exit_code, Some(3), "{r:?}");
    let r = s
        .run_command(&actor(), req("ls /definitely_missing_dir_aht"), &ap)
        .await
        .unwrap();
    // GNU ls exits 2 for a missing operand, BSD ls (macOS) exits 1.
    assert!(matches!(r.exit_code, Some(1 | 2)), "{r:?}");
    assert!(r.output.contains("No such file"), "{r:?}");
    let r = s
        .run_command(&actor(), req("printf 'a\\nb\\n'; echo c"), &ap)
        .await
        .unwrap();
    assert_eq!(
        r.output.trim().lines().collect::<Vec<_>>(),
        ["a", "b", "c"],
        "{r:?}"
    );
    let r = s
        .run_command(&actor(), req("cd / && pwd"), &ap)
        .await
        .unwrap();
    assert_eq!(r.output.trim(), "/");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(s.info().cwd.as_deref(), Some("/"));

    let ui = String::from_utf8_lossy(&ui.lock().unwrap()).into_owned();
    for leak in ["AHTPROBE", "Aht", "__aht"] {
        assert!(!ui.contains(leak), "UI leaked {leak:?}");
    }
    assert!(ui.contains("\x1b]133;A"), "prompt marks reach the UI");
    let _ = s.close().await;
}

/// zsh is the default shell on macOS. `-f` skips the user's rc files, like `--norc` for bash.
#[tokio::test(flavor = "multi_thread")]
async fn real_zsh_hook_marks_and_exit_codes() {
    if !on_path("zsh") {
        eprintln!("zsh not on PATH; skipping");
        return;
    }
    let (s, ui) = start("zsh", &["-f", "-i"]).await;
    assert_eq!(
        s.integration(),
        IntegrationMode::Full,
        "hook did not install"
    );
    let info = s.info();
    assert_eq!(info.shell.as_deref(), Some("zsh"));
    assert!(
        info.os.is_some() && info.user.is_some() && info.host.is_some(),
        "{info:?}"
    );
    assert!(info.cwd.is_some(), "OSC 7 cwd: {info:?}");

    let ap = TestApprover::approve();
    let r = s
        .run_command(&actor(), req("echo hello"), &ap)
        .await
        .unwrap();
    assert_eq!(r.state, CommandState::Completed, "{r:?}");
    assert_eq!(r.exit_code, Some(0));
    assert_eq!(r.output.trim(), "hello");

    let r = s.run_command(&actor(), req("false"), &ap).await.unwrap();
    assert_eq!(r.exit_code, Some(1), "{r:?}");
    let r = s.run_command(&actor(), req("(exit 3)"), &ap).await.unwrap();
    assert_eq!(r.exit_code, Some(3), "{r:?}");
    let r = s
        .run_command(&actor(), req("ls /definitely_missing_dir_aht"), &ap)
        .await
        .unwrap();
    assert!(matches!(r.exit_code, Some(1 | 2)), "{r:?}");
    assert!(r.output.contains("No such file"), "{r:?}");
    let r = s
        .run_command(&actor(), req("printf 'a\\nb\\n'; echo c"), &ap)
        .await
        .unwrap();
    assert_eq!(
        r.output.trim().lines().collect::<Vec<_>>(),
        ["a", "b", "c"],
        "{r:?}"
    );
    let r = s
        .run_command(&actor(), req("cd / && pwd"), &ap)
        .await
        .unwrap();
    assert_eq!(r.output.trim(), "/");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(s.info().cwd.as_deref(), Some("/"));

    let ui = String::from_utf8_lossy(&ui.lock().unwrap()).into_owned();
    for leak in ["AHTPROBE", "Aht", "__aht"] {
        assert!(!ui.contains(leak), "UI leaked {leak:?}");
    }
    assert!(ui.contains("\x1b]133;A"), "prompt marks reach the UI");
    let _ = s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn real_pwsh_hook_marks_and_exit_codes() {
    if !on_path("pwsh") {
        eprintln!("pwsh not on PATH; skipping");
        return;
    }
    let (s, _ui) = start("pwsh", &["-NoLogo", "-NoProfile"]).await;
    assert_eq!(s.info().shell.as_deref(), Some("powershell"));
    assert_eq!(
        s.integration(),
        IntegrationMode::Full,
        "hook did not install"
    );
    let ap = TestApprover::approve();
    let r = s
        .run_command(&actor(), req("echo hello"), &ap)
        .await
        .unwrap();
    assert_eq!(
        (r.state, r.exit_code),
        (CommandState::Completed, Some(0)),
        "{r:?}"
    );
    assert_eq!(r.output.trim(), "hello");
    // A native child process's exit code, on every OS (`cmd` exists only on Windows).
    let r = s
        .run_command(
            &actor(),
            req("pwsh -NoLogo -NoProfile -NonInteractive -Command 'exit 4'"),
            &ap,
        )
        .await
        .unwrap();
    assert_eq!(r.exit_code, Some(4), "{r:?}");
    let _ = s.close().await;
}

async fn wait_idle_full(s: &TermSession) {
    for _ in 0..200 {
        if s.state() == SessionState::Idle && s.integration() == IntegrationMode::Full {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The user switches into a nested shell (like `sudo su -` on a server); the agent must follow
/// into it, and back out again after `exit`.
#[tokio::test(flavor = "multi_thread")]
async fn real_bash_nested_shell_is_followed() {
    if !on_path("bash") {
        eprintln!("bash not on PATH; skipping");
        return;
    }
    let (s, ui) = start("bash", &["--norc", "-i"]).await;
    assert_eq!(
        s.integration(),
        IntegrationMode::Full,
        "hook did not install"
    );
    let ap = TestApprover::approve();
    let outer = s
        .run_command(&actor(), req("echo lvl=$SHLVL"), &ap)
        .await
        .unwrap();
    let outer_lvl: u32 = outer
        .output
        .trim()
        .trim_start_matches("lvl=")
        .parse()
        .unwrap();

    // The user (not the agent) starts a nested shell.
    s.write_user(b"bash --norc -i\r").await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    wait_idle_full(&s).await;
    assert_eq!(s.state(), SessionState::Idle, "nested shell not detected");
    let inner = s
        .run_command(&actor(), req("echo lvl=$SHLVL"), &ap)
        .await
        .unwrap();
    assert_eq!(inner.state, CommandState::Completed, "{inner:?}");
    assert_eq!(inner.output.trim(), format!("lvl={}", outer_lvl + 1));
    assert_eq!(inner.exit_code, Some(0));

    // Back to the outer shell.
    s.write_user(b"exit\r").await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    wait_idle_full(&s).await;
    let back = s
        .run_command(&actor(), req("echo lvl=$SHLVL"), &ap)
        .await
        .unwrap();
    assert_eq!(back.output.trim(), format!("lvl={outer_lvl}"), "{back:?}");
    assert_eq!(s.info().shell.as_deref(), Some("bash"));

    let shown = String::from_utf8_lossy(&ui.lock().unwrap()).to_string();
    assert!(!shown.contains("AHTPROBE"), "probe leaked to the UI");
    assert!(!shown.contains("__aht_pc"), "hook leaked to the UI");
}
