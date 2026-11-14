//! Headless supervisor: spawns a [`LaunchSpec`], feeds the prompt, parses stdout into
//! [`AgentEvent`]s and tears the whole process tree down on cancel.
//!
//! Prompt delivery by harness: Claude Code gets a stream-json user message on stdin (stdin stays
//! open for follow-ups via [`HeadlessRun::send_user_message`]); Codex reads the prompt from stdin
//! (spec ends with `-`) and stdin is closed; OpenCode takes the prompt as its final argv entry.
//!
//! Every turn (the first prompt and each follow-up) ends with a `Done`: when the process exits
//! while a turn is still open, the runner sends it, after an `Error` for a non-zero exit (not
//! caused by `cancel`).

use crate::detect::{command_for, resolve_program};
use crate::launch::LaunchSpec;
use crate::stream::{parser_for, StreamParser};
use aht_core::agent::AgentEvent;
use aht_core::model::HarnessKind;
use aht_core::{Error, Result};
use serde_json::json;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::ChildStdin;
use tokio::sync::{mpsc, watch, Mutex, Notify};

/// Max stderr lines forwarded as `Error` events per run (noisy CLIs).
const MAX_STDERR_EVENTS: usize = 200;
/// Longest stdout/stderr line read from a harness; a longer one ends the stream with an error.
const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;
const EVENT_CHANNEL: usize = 256;

/// A CLI that exits before reading its prompt (bad flag, not signed in) closes its stdin. Its
/// stderr and exit status, reported by the reader tasks, explain why; a broken pipe on the
/// prompt write would only hide that.
fn tolerate_exit(r: std::io::Result<()>) -> std::io::Result<()> {
    match r {
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        other => other,
    }
}

/// Handle to a running headless harness process.
#[derive(Clone)]
pub struct HeadlessRun {
    kind: HarnessKind,
    /// Turns not yet ended by `Done` (see `spawn`).
    open_turns: Arc<AtomicUsize>,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    cancel: Arc<Notify>,
    finished: watch::Receiver<bool>,
    pid: Option<u32>,
}

impl HeadlessRun {
    /// Write the spec's temp files, spawn the process and start streaming events.
    pub async fn start(
        spec: LaunchSpec,
        initial_prompt: &str,
    ) -> Result<(HeadlessRun, mpsc::Receiver<AgentEvent>)> {
        let written = write_files(&spec)?;
        match Self::spawn(spec, initial_prompt, written.clone()).await {
            Ok(r) => Ok(r),
            Err(e) => {
                cleanup(&written);
                Err(e)
            }
        }
    }

    async fn spawn(
        spec: LaunchSpec,
        prompt: &str,
        files: Vec<PathBuf>,
    ) -> Result<(HeadlessRun, mpsc::Receiver<AgentEvent>)> {
        let program = resolve_program(&spec.program);
        let mut cmd = command_for(&program);
        cmd.args(&spec.args);
        if spec.kind == HarnessKind::OpenCode {
            cmd.arg(opencode_prompt_arg(prompt));
        }
        for k in &spec.env_remove {
            cmd.env_remove(k);
        }
        cmd.envs(spec.env.iter().map(|(k, v)| (k, v)));
        if let Some(cwd) = &spec.cwd {
            std::fs::create_dir_all(cwd)?;
            cmd.current_dir(cwd);
        }
        cmd.stdin(if spec.kind == HarnessKind::OpenCode {
            Stdio::null()
        } else {
            Stdio::piped()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);

        let mut child = cmd
            .spawn()
            .map_err(|e| Error::Io(format!("failed to start {}: {e}", program.display())))?;
        let pid = child.id();
        let mut stdin = child.stdin.take();

        match spec.kind {
            HarnessKind::ClaudeCode => {
                if let Some(si) = stdin.as_mut() {
                    tolerate_exit(si.write_all(user_message_line(prompt).as_bytes()).await)?;
                    tolerate_exit(si.flush().await)?;
                }
            }
            HarnessKind::Codex => {
                if let Some(mut si) = stdin.take() {
                    tolerate_exit(si.write_all(prompt.as_bytes()).await)?;
                    tolerate_exit(si.shutdown().await)?;
                }
            }
            HarnessKind::OpenCode => {}
        }

        let (tx, rx) = mpsc::channel(EVENT_CHANNEL);
        let cancel = Arc::new(Notify::new());
        let (fin_tx, fin_rx) = watch::channel(false);
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));

        // Turns asked for and not yet ended by a `Done`: the first prompt, plus one per
        // follow-up. A process that exits with one open owes that turn its Error + Done.
        let open_turns = Arc::new(AtomicUsize::new(1));
        let stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");
        let out_task = tokio::spawn(pump_stdout(
            stdout,
            parser_for(spec.kind),
            tx.clone(),
            open_turns.clone(),
        ));
        let err_task = tokio::spawn(pump_stderr(stderr, tx.clone()));

        let turns_handle = open_turns.clone();
        let waiter_cancel = cancel.clone();
        let waiter_cancelled = cancelled.clone();
        tokio::spawn(async move {
            let status = tokio::select! {
                s = child.wait() => s.ok(),
                () = waiter_cancel.notified() => {
                    waiter_cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
                    if let Some(pid) = pid { kill_tree(pid).await; }
                    let _ = child.kill().await;
                    child.wait().await.ok()
                }
            };
            // Let readers drain; a grandchild holding the pipes open must not hang us.
            let _ = tokio::time::timeout(Duration::from_secs(3), out_task).await;
            let _ = tokio::time::timeout(Duration::from_secs(1), err_task).await;
            let done_seen = open_turns.load(Ordering::SeqCst) == 0;
            let was_cancelled = waiter_cancelled.load(std::sync::atomic::Ordering::SeqCst);
            if !was_cancelled && !done_seen {
                if let Some(s) = status.filter(|s| !s.success()) {
                    let _ = tx
                        .send(AgentEvent::Error {
                            message: format!("process exited with {s}"),
                            retryable: true,
                        })
                        .await;
                }
            }
            if !done_seen {
                let _ = tx.send(AgentEvent::Done).await;
            }
            cleanup(&files);
            drop(tx);
            let _ = fin_tx.send(true);
        });

        Ok((
            HeadlessRun {
                kind: spec.kind,
                open_turns: turns_handle,
                stdin: Arc::new(Mutex::new(stdin)),
                cancel,
                finished: fin_rx,
                pid,
            },
            rx,
        ))
    }

    /// Send a follow-up user turn. Only Claude Code supports stream-json input.
    pub async fn send_user_message(&self, text: &str) -> Result<()> {
        if self.kind != HarnessKind::ClaudeCode {
            return Err(Error::Invalid(
                "follow-up messages are only supported for Claude Code stream-json input".into(),
            ));
        }
        let mut guard = self.stdin.lock().await;
        let si = guard
            .as_mut()
            .ok_or_else(|| Error::Busy("harness stdin is closed".into()))?;
        self.open_turns.fetch_add(1, Ordering::SeqCst);
        if let Err(e) = si.write_all(user_message_line(text).as_bytes()).await {
            end_turn(&self.open_turns);
            return Err(e.into());
        }
        si.flush().await?;
        Ok(())
    }

    /// Kill the process tree and wait (bounded) for cleanup to finish.
    pub async fn cancel(&self) {
        self.cancel.notify_one();
        self.wait().await;
    }

    /// Resolve once the process has exited and temp files are removed (bounded to 10 s).
    pub async fn wait(&self) {
        let mut rx = self.finished.clone();
        let _ = tokio::time::timeout(Duration::from_secs(10), rx.wait_for(|f| *f)).await;
    }

    pub fn is_finished(&self) -> bool {
        *self.finished.borrow()
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }
}

/// OpenCode takes the prompt as a positional argument. A prompt starting with `-` would be read
/// as an option, so it gets a leading space (harmless to the model).
// shortcut: not `--`, since whether `opencode run` accepts it is unverified; switch once checked.
fn opencode_prompt_arg(prompt: &str) -> String {
    if prompt.starts_with('-') {
        format!(" {prompt}")
    } else {
        prompt.to_string()
    }
}

/// One Claude stream-json user message (`--input-format stream-json`), newline terminated.
/// Shape from the Agent SDK streaming-input docs [U: verify with --replay-user-messages].
pub fn user_message_line(text: &str) -> String {
    let mut s = json!({
        "type": "user",
        "message": {"role": "user", "content": [{"type": "text", "text": text}]}
    })
    .to_string();
    s.push('\n');
    s
}

/// One open turn ended (never below zero: a stray `Done` must not hide a later turn).
fn end_turn(open: &AtomicUsize) {
    let mut cur = open.load(Ordering::SeqCst);
    while cur > 0 {
        match open.compare_exchange(cur, cur - 1, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return,
            Err(now) => cur = now,
        }
    }
}

/// Read one `\n`-terminated line of at most [`MAX_LINE_BYTES`] (invalid UTF-8 replaced).
/// `Ok(None)` at EOF; an error for an overlong line or a read failure.
async fn next_line<R: tokio::io::AsyncBufRead + Unpin>(
    r: &mut R,
) -> std::io::Result<Option<String>> {
    let mut buf = Vec::new();
    // `take` bounds what one call may buffer; +1 tells a full line from an overlong one.
    let n = (&mut *r)
        .take(MAX_LINE_BYTES as u64 + 1)
        .read_until(b'\n', &mut buf)
        .await?;
    if n == 0 {
        return Ok(None);
    }
    if buf.len() > MAX_LINE_BYTES {
        return Err(std::io::Error::other(format!(
            "output line longer than {} MB",
            MAX_LINE_BYTES / (1024 * 1024)
        )));
    }
    while matches!(buf.last(), Some(b'\n' | b'\r')) {
        buf.pop();
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

async fn pump_stdout(
    out: tokio::process::ChildStdout,
    mut parser: Box<dyn StreamParser>,
    tx: mpsc::Sender<AgentEvent>,
    open_turns: Arc<AtomicUsize>,
) {
    let mut out = BufReader::new(out);
    loop {
        let line = match next_line(&mut out).await {
            Ok(Some(l)) => l,
            Ok(None) => return,
            Err(e) => {
                let _ = tx
                    .send(AgentEvent::Error {
                        message: format!("harness output unreadable: {e}"),
                        retryable: false,
                    })
                    .await;
                return;
            }
        };
        for ev in parser.feed_line(&line) {
            if ev == AgentEvent::Done {
                end_turn(&open_turns);
            }
            if tx.send(ev).await.is_err() {
                return; // receiver dropped
            }
        }
    }
}

async fn pump_stderr(err: tokio::process::ChildStderr, tx: mpsc::Sender<AgentEvent>) {
    let mut err = BufReader::new(err);
    let mut sent = 0;
    while let Ok(Some(line)) = next_line(&mut err).await {
        let line = line.trim();
        if line.is_empty() || sent >= MAX_STDERR_EVENTS {
            continue;
        }
        sent += 1;
        let ev = AgentEvent::Error {
            message: line.to_string(),
            retryable: false,
        };
        if tx.send(ev).await.is_err() {
            return;
        }
    }
}

/// Write the spec's files; returns the paths actually written (for cleanup).
pub fn write_files(spec: &LaunchSpec) -> Result<Vec<PathBuf>> {
    let mut written = vec![];
    for (path, content) in &spec.files {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if let Err(e) = std::fs::write(path, content) {
            cleanup(&written);
            return Err(e.into());
        }
        written.push(path.clone());
    }
    Ok(written)
}

/// Remove temp files and their parent dir if it is left empty.
pub fn cleanup(files: &[PathBuf]) {
    for f in files {
        let _ = std::fs::remove_file(f);
        if let Some(parent) = f.parent() {
            let _ = std::fs::remove_dir(parent); // only succeeds when empty
        }
    }
}

/// Kill `pid` and all of its descendants.
pub async fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        let mut c = tokio::process::Command::new("taskkill");
        c.args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x0800_0000);
        let _ = c.status().await;
    }
    #[cfg(unix)]
    {
        // The child leads its own process group (process_group(0)); signal the whole group.
        let _ = tokio::process::Command::new("kill")
            .args(["-KILL", "--", &format!("-{pid}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_message_shape() {
        let l = user_message_line("hi \"there\"\nline2");
        assert!(l.ends_with('\n'));
        assert_eq!(l.matches('\n').count(), 1, "must be a single JSON line");
        let v: serde_json::Value = serde_json::from_str(l.trim()).unwrap();
        assert_eq!(v["type"], "user");
        assert_eq!(v["message"]["content"][0]["text"], "hi \"there\"\nline2");
    }

    #[test]
    fn opencode_prompt_never_looks_like_an_option() {
        assert_eq!(opencode_prompt_arg("check disk"), "check disk");
        assert_eq!(opencode_prompt_arg("--help me"), " --help me");
        assert_eq!(opencode_prompt_arg("-rf"), " -rf");
    }

    #[tokio::test]
    async fn next_line_caps_line_length() {
        let data = b"one\r\ntwo\n\xff\n".to_vec();
        let mut r = &data[..];
        assert_eq!(next_line(&mut r).await.unwrap().as_deref(), Some("one"));
        assert_eq!(next_line(&mut r).await.unwrap().as_deref(), Some("two"));
        assert_eq!(
            next_line(&mut r).await.unwrap().as_deref(),
            Some("\u{fffd}")
        );
        assert_eq!(next_line(&mut r).await.unwrap(), None);
        let long = vec![b'a'; MAX_LINE_BYTES + 10];
        let mut r = &long[..];
        assert!(next_line(&mut r).await.is_err());
    }

    #[test]
    fn cleanup_removes_files_and_empty_dir() {
        let d = tempfile::tempdir().unwrap();
        let sub = d.path().join(".aht");
        std::fs::create_dir_all(&sub).unwrap();
        let f = sub.join("a.json");
        std::fs::write(&f, "x").unwrap();
        cleanup(std::slice::from_ref(&f));
        assert!(!f.exists());
        assert!(!sub.exists());
    }
}
