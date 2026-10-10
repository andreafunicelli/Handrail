//! Runner tests using the `fake_harness` helper binary (feature `test-support`).

use aht_core::agent::AgentEvent;
use aht_core::model::HarnessKind;
use aht_harness::{HeadlessRun, LaunchMode, LaunchSpec};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::mpsc::Receiver;

fn fake() -> String {
    env!("CARGO_BIN_EXE_fake_harness").to_string()
}

fn fixture(name: &str) -> String {
    format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn spec(kind: HarnessKind, args: Vec<String>, cwd: &Path) -> LaunchSpec {
    LaunchSpec {
        kind,
        mode: LaunchMode::Headless,
        program: fake(),
        args,
        env: vec![],
        env_remove: vec![],
        cwd: Some(cwd.to_path_buf()),
        files: vec![],
    }
}

async fn collect(mut rx: Receiver<AgentEvent>) -> Vec<AgentEvent> {
    let mut out = vec![];
    let all = async {
        while let Some(e) = rx.recv().await {
            out.push(e);
        }
    };
    tokio::time::timeout(Duration::from_secs(20), all)
        .await
        .expect("event stream did not end");
    out
}

fn pid_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        let out = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
    }
    #[cfg(unix)]
    {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .unwrap()
            .success()
    }
}

#[tokio::test]
async fn codex_replay_prompt_on_stdin_files_and_stderr() {
    let tmp = tempfile::tempdir().unwrap();
    let stdin_file = tmp.path().join("stdin.txt");
    let mut s = spec(
        HarnessKind::Codex,
        vec![
            fixture("codex_exec.jsonl"),
            format!("--stdin-to={}", stdin_file.display()),
            "--stderr=warning: something".into(),
        ],
        tmp.path(),
    );
    let temp_cfg: PathBuf = tmp.path().join(".aht").join("cfg.json");
    s.files.push((temp_cfg.clone(), "{}".into()));

    let (run, rx) = HeadlessRun::start(s, "hello\nworld \"q\"").await.unwrap();
    assert!(temp_cfg.exists(), "temp file written before spawn");
    let ev = collect(rx).await;
    run.wait().await;

    assert_eq!(
        std::fs::read_to_string(&stdin_file).unwrap(),
        "hello\nworld \"q\""
    );
    assert!(ev.contains(&AgentEvent::TextDelta {
        text: "Disk is 40% used.".into()
    }));
    assert!(ev
        .iter()
        .any(|e| matches!(e, AgentEvent::ToolCall { name, .. } if name == "ssh_exec")));
    assert!(ev.contains(&AgentEvent::Error {
        message: "warning: something".into(),
        retryable: false
    }));
    assert_eq!(ev.iter().filter(|e| **e == AgentEvent::Done).count(), 1);
    assert!(!temp_cfg.exists(), "temp file removed");
    assert!(run.is_finished());
}

#[tokio::test]
async fn opencode_prompt_is_last_argv() {
    let tmp = tempfile::tempdir().unwrap();
    let args_file = tmp.path().join("args.txt");
    let s = spec(
        HarnessKind::OpenCode,
        vec![
            fixture("opencode_run.jsonl"),
            format!("--args-to={}", args_file.display()),
        ],
        tmp.path(),
    );
    let (run, rx) = HeadlessRun::start(s, "check disk").await.unwrap();
    let ev = collect(rx).await;
    run.wait().await;
    let args = std::fs::read_to_string(&args_file).unwrap();
    assert_eq!(args.lines().last(), Some("check disk"));
    assert_eq!(ev.last(), Some(&AgentEvent::Done));
    assert!(run.send_user_message("x").await.is_err(), "claude-only");
}

#[tokio::test]
async fn env_set_and_removed() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("AHT_TEST_SCRUB_ME", "leaky");
    let mut s = spec(
        HarnessKind::Codex,
        vec!["--print-env=AHT_TEST_SCRUB_ME".into()],
        tmp.path(),
    );
    s.env_remove.push("AHT_TEST_SCRUB_ME".into());
    s.env.push(("AHT_TEST_INJECTED".into(), "tok".into()));
    let (_run, rx) = HeadlessRun::start(s, "p").await.unwrap();
    let ev = collect(rx).await;
    assert!(
        ev.contains(&AgentEvent::Error {
            message: "ENV AHT_TEST_SCRUB_ME=unset".into(),
            retryable: false
        }),
        "{ev:?}"
    );
}

#[tokio::test]
async fn injected_env_reaches_child() {
    let tmp = tempfile::tempdir().unwrap();
    let mut s = spec(
        HarnessKind::Codex,
        vec!["--print-env=AHT_TEST_INJECTED".into()],
        tmp.path(),
    );
    s.env.push(("AHT_TEST_INJECTED".into(), "tok".into()));
    let (_run, rx) = HeadlessRun::start(s, "p").await.unwrap();
    let ev = collect(rx).await;
    assert!(ev.contains(&AgentEvent::Error {
        message: "ENV AHT_TEST_INJECTED=tok".into(),
        retryable: false
    }));
}

#[tokio::test]
async fn nonzero_exit_reports_error_then_done() {
    let tmp = tempfile::tempdir().unwrap();
    let s = spec(HarnessKind::Codex, vec!["--exit=3".into()], tmp.path());
    let (_run, rx) = HeadlessRun::start(s, "p").await.unwrap();
    let ev = collect(rx).await;
    assert_eq!(ev.len(), 2, "{ev:?}");
    assert!(matches!(
        &ev[0],
        AgentEvent::Error {
            retryable: true,
            ..
        }
    ));
    assert_eq!(ev[1], AgentEvent::Done);
}

#[tokio::test]
async fn claude_follow_up_messages() {
    let tmp = tempfile::tempdir().unwrap();
    let s = spec(
        HarnessKind::ClaudeCode,
        vec!["--claude-echo".into()],
        tmp.path(),
    );
    let (run, mut rx) = HeadlessRun::start(s, "first").await.unwrap();

    async fn until_done(rx: &mut Receiver<AgentEvent>) -> Vec<AgentEvent> {
        let mut v = vec![];
        loop {
            let e = tokio::time::timeout(Duration::from_secs(20), rx.recv())
                .await
                .expect("timeout")
                .expect("closed early");
            let done = e == AgentEvent::Done;
            v.push(e);
            if done {
                return v;
            }
        }
    }
    let first = until_done(&mut rx).await;
    assert!(first.contains(&AgentEvent::TextDelta {
        text: "echo: first".into()
    }));
    run.send_user_message("second").await.unwrap();
    let second = until_done(&mut rx).await;
    assert!(second.contains(&AgentEvent::TextDelta {
        text: "echo: second".into()
    }));
    run.cancel().await;
}

#[tokio::test]
async fn cancel_kills_process_tree_and_cleans_up() {
    let tmp = tempfile::tempdir().unwrap();
    let mut s = spec(
        HarnessKind::Codex,
        vec!["--spawn-child".into(), "--hang".into()],
        tmp.path(),
    );
    let temp_cfg = tmp.path().join(".aht").join("c.json");
    s.files.push((temp_cfg.clone(), "{}".into()));
    let (run, mut rx) = HeadlessRun::start(s, "p").await.unwrap();

    // The fake reports its grandchild's pid via an `error` JSON line, which the codex parser
    // surfaces as an Error event.
    let child_pid: u32 = loop {
        let e = tokio::time::timeout(Duration::from_secs(20), rx.recv())
            .await
            .expect("timeout")
            .expect("closed");
        if let AgentEvent::Error { message, .. } = e {
            if let Some(p) = message.strip_prefix("CHILD ") {
                break p.parse().unwrap();
            }
        }
    };
    let pid = run.pid().unwrap();
    assert!(pid_alive(pid) && pid_alive(child_pid));

    run.cancel().await;
    assert!(run.is_finished());
    let rest = collect(rx).await;
    assert!(rest.contains(&AgentEvent::Done));
    assert!(!rest.iter().any(|e| matches!(
        e,
        AgentEvent::Error {
            retryable: true,
            ..
        }
    )));
    assert!(!temp_cfg.exists());
    // Allow the OS a moment to reap.
    for _ in 0..50 {
        if !pid_alive(pid) && !pid_alive(child_pid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("process tree still alive");
}

#[tokio::test]
async fn spawn_failure_cleans_files() {
    let tmp = tempfile::tempdir().unwrap();
    let mut s = spec(HarnessKind::Codex, vec![], tmp.path());
    s.program = tmp
        .path()
        .join("does-not-exist")
        .to_string_lossy()
        .into_owned();
    let f = tmp.path().join(".aht").join("x.json");
    s.files.push((f.clone(), "{}".into()));
    assert!(HeadlessRun::start(s, "p").await.is_err());
    assert!(!f.exists());
}
