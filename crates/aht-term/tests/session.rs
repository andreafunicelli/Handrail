mod common;

use aht_core::agent::{ApprovalDecision, RiskClass};
use aht_core::model::{ApprovalMode, Environment};
use aht_core::terminal::{
    Actor, CommandState, ControlKey, IntegrationMode, RunCommandRequest, SessionState,
};
use aht_core::Error;
use aht_term::{AuditDecision, Policy, SessionEvent, TermConfig, TermSession};
use common::{FakeHandle, FakeShell, Kind, TestApprover};
use regex::Regex;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

fn actor() -> Actor {
    Actor {
        id: "test:agent".into(),
        label: "Test Agent".into(),
        session_scope: None,
    }
}

fn auto() -> Policy {
    Policy {
        mode: ApprovalMode::Auto,
        env: Environment::Dev,
        deny_list: vec![],
    }
}

struct Ctx {
    s: TermSession,
    sh: Arc<FakeShell>,
    ui: broadcast::Receiver<Vec<u8>>,
}

impl Ctx {
    fn ui_text(&mut self) -> String {
        let mut out = Vec::new();
        while let Ok(c) = self.ui.try_recv() {
            out.extend(c);
        }
        String::from_utf8_lossy(&out).into_owned()
    }
}

async fn start(kind: Kind, policy: Policy) -> Ctx {
    let (sh, rx) = FakeShell::new(kind);
    let s = TermSession::spawn(
        Arc::new(FakeHandle(sh.clone())),
        rx,
        TermConfig {
            policy,
            ..Default::default()
        },
    );
    let ui = s.subscribe_ui();
    for _ in 0..400 {
        if s.integration() != IntegrationMode::Unknown && s.state() == SessionState::Idle {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(s.state(), SessionState::Idle, "session never became idle");
    Ctx { s, sh, ui }
}

fn req(cmd: &str) -> RunCommandRequest {
    RunCommandRequest {
        session_id: aht_core::SessionId::new(),
        command: cmd.into(),
        timeout_s: Some(30),
        reason: None,
    }
}

fn req_t(cmd: &str, t: u64) -> RunCommandRequest {
    RunCommandRequest {
        timeout_s: Some(t),
        ..req(cmd)
    }
}

#[tokio::test(start_paused = true)]
async fn full_mode_injection_is_hidden_from_ui() {
    let mut c = start(Kind::Bash, auto()).await;
    assert_eq!(c.s.integration(), IntegrationMode::Full);
    assert!(c.sh.hooked());
    let ui = c.ui_text();
    assert!(ui.contains("Welcome to FakeOS"));
    for leak in ["AHTPROBE", "Aht", "__aht", "133;C", "printf"] {
        assert!(!ui.contains(leak), "UI leaked {leak:?}: {ui:?}");
    }
    // After the hidden window the line is cleared and the fresh prompt drawn.
    assert!(ui.contains("\r\x1b[K"), "{ui:?}");
    assert!(ui.ends_with("\x1b]133;B\x07"), "{ui:?}");
    let info = c.s.info();
    assert_eq!(info.shell.as_deref(), Some("bash"));
    assert_eq!(info.os.as_deref(), Some("FakeOS 1.0"));
    assert_eq!(info.user.as_deref(), Some("u"));
    assert_eq!(info.host.as_deref(), Some("fake"));
    assert_eq!(info.cwd.as_deref(), Some("/home/u"));
}

#[tokio::test(start_paused = true)]
async fn full_mode_run_and_exit_codes() {
    let mut c = start(Kind::Bash, auto()).await;
    let mut ev = c.s.subscribe_events();
    let ap = TestApprover::approve();
    let r =
        c.s.run_command(&actor(), req("echo hello world"), &ap)
            .await
            .unwrap();
    assert_eq!(r.state, CommandState::Completed);
    assert_eq!(r.exit_code, Some(0));
    assert_eq!(r.output, "hello world\n");
    assert!(!r.truncated);
    let ui = c.ui_text();
    assert!(
        ui.contains("echo hello world"),
        "agent command is visible in the UI"
    );

    let r = c.s.run_command(&actor(), req("false"), &ap).await.unwrap();
    assert_eq!((r.state, r.exit_code), (CommandState::Completed, Some(1)));
    let r =
        c.s.run_command(&actor(), req("nosuch --x"), &ap)
            .await
            .unwrap();
    assert_eq!(r.exit_code, Some(127));
    assert_eq!(r.output, "bash: nosuch: command not found\n");
    let r = c.s.run_command(&actor(), req("ls"), &ap).await.unwrap();
    assert_eq!(r.output, "a.txt  b.txt\n", "ANSI stripped");
    let r =
        c.s.run_command(&actor(), req("printf 'no newline'"), &ap)
            .await
            .unwrap();
    assert_eq!(r.output, "no newline");
    assert_eq!(ap.count(), 0, "auto mode never asks for these");

    let mut marks = 0;
    while let Ok(e) = ev.try_recv() {
        if let SessionEvent::AgentMark { actor_label, .. } = e {
            assert_eq!(actor_label, "Test Agent");
            marks += 1;
        }
    }
    assert_eq!(marks, 5);
    let audit = c.s.audit_log();
    assert_eq!(audit.len(), 5);
    assert_eq!(audit[1].exit_code, Some(1));
    assert_eq!(audit[0].decision, AuditDecision::AutoApproved);
    assert_eq!(audit[0].actor_id, "test:agent");
}

#[tokio::test(start_paused = true)]
async fn limited_mode_run() {
    let mut c = start(Kind::Dumb, auto()).await;
    assert_eq!(c.s.integration(), IntegrationMode::Limited);
    assert_eq!(c.s.info().shell.as_deref(), Some("sh"));
    let ui = c.ui_text();
    assert!(!ui.contains("AHTPROBE"), "{ui:?}");
    assert!(ui.ends_with("\r\x1b[K$ "), "prompt redrawn: {ui:?}");
    let ap = TestApprover::approve();
    let r =
        c.s.run_command(&actor(), req("echo hi there"), &ap)
            .await
            .unwrap();
    assert_eq!(r.state, CommandState::Completed);
    assert_eq!(r.exit_code, None);
    assert_eq!(r.output, "hi there\n");
    assert!(r.duration_ms >= 1500, "quiescence-based end");
    let r = c.s.run_command(&actor(), req("ls"), &ap).await.unwrap();
    assert_eq!(r.output, "a.txt  b.txt\n");
    assert_eq!(c.s.state(), SessionState::Idle);
}

#[tokio::test(start_paused = true)]
async fn truncation_and_paging() {
    let c = start(Kind::Bash, auto()).await;
    let ap = TestApprover::approve();
    let r =
        c.s.run_command(&actor(), req("cat bigfile"), &ap)
            .await
            .unwrap();
    assert_eq!(r.state, CommandState::Completed);
    assert!(r.truncated);
    assert!(r.output.len() < 8192 + 200);
    assert!(r.output.starts_with("line 000000 "));
    assert!(r.output.contains("omitted"));
    let id = r.output_id.unwrap();
    let mut all = String::new();
    let mut off = 0;
    loop {
        let ch = c.s.read_output(id, off, 32768).unwrap();
        all.push_str(&ch.data);
        off += ch.data.chars().count();
        if ch.complete {
            assert_eq!(off, ch.total_len);
            break;
        }
        assert!(!ch.data.is_empty());
    }
    assert_eq!(
        all.lines().count(),
        1897,
        "every line of the ~100 KB output"
    );
    let line_re = Regex::new(r"^line \d{6} x{40}$").unwrap();
    assert!(
        all.lines().all(|l| line_re.is_match(l)),
        "no echo / prompt in output"
    );
}

#[tokio::test(start_paused = true)]
async fn timeout_then_completion() {
    let c = start(Kind::Bash, auto()).await;
    let ap = TestApprover::approve();
    let r =
        c.s.run_command(&actor(), req_t("sleep 5", 1), &ap)
            .await
            .unwrap();
    assert_eq!(r.state, CommandState::TimedOut);
    let id = r.output_id.unwrap();
    assert!(!c.s.read_output(id, 0, 100).unwrap().complete);
    // A second agent command must wait for the prompt; with a short timeout it is refused.
    let e =
        c.s.run_command(&actor(), req_t("echo x", 1), &ap)
            .await
            .unwrap_err();
    assert!(matches!(e, Error::Busy(_)), "{e:?}");
    tokio::time::sleep(Duration::from_secs(5)).await;
    let ch = c.s.read_output(id, 0, 100).unwrap();
    assert!(ch.complete);
    assert_eq!(ch.data, "slept\n");
    assert_eq!(c.s.state(), SessionState::Idle);
}

#[tokio::test(start_paused = true)]
async fn needs_user_on_sudo_password() {
    let c = start(Kind::Bash, auto()).await;
    let ap = TestApprover::approve();
    let r =
        c.s.run_command(&actor(), req("sudo echo secret-ok"), &ap)
            .await
            .unwrap();
    assert_eq!(r.state, CommandState::NeedsUser);
    assert!(r.note.unwrap().contains("[sudo]"));
    assert!(r.output.contains("[sudo] password for u:"));
    // The user types the password; the agent never sees it in its result.
    c.s.write_user(b"hunter2\r").await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let ch = c.s.read_output(r.output_id.unwrap(), 0, 1000).unwrap();
    assert!(ch.complete);
    assert!(ch.data.contains("secret-ok"));
    assert!(!ch.data.contains("hunter2"));
    assert_eq!(c.s.state(), SessionState::Idle);
}

#[tokio::test(start_paused = true)]
async fn needs_user_on_full_screen() {
    let c = start(Kind::Bash, auto()).await;
    let ap = TestApprover::approve();
    let r =
        c.s.run_command(&actor(), req("vim notes"), &ap)
            .await
            .unwrap();
    assert_eq!(r.state, CommandState::NeedsUser);
    assert!(r.note.unwrap().contains("full-screen"));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(c.s.state(), SessionState::FullScreen);
    let e =
        c.s.run_command(&actor(), req("echo x"), &ap)
            .await
            .unwrap_err();
    assert!(
        matches!(e, Error::Busy(ref m) if m.contains("full-screen")),
        "{e:?}"
    );
    c.s.write_user(b":q\r").await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(c.s.state(), SessionState::Idle);
    let r =
        c.s.run_command(&actor(), req("echo back"), &ap)
            .await
            .unwrap();
    assert_eq!(r.output, "back\n");
}

#[tokio::test(start_paused = true)]
async fn user_takeover_interrupts_agent() {
    let c = start(Kind::Bash, auto()).await;
    let s2 = c.s.clone();
    let h = tokio::spawn(async move {
        let ap = TestApprover::approve();
        s2.run_command(&actor(), req("sleep 3"), &ap).await.unwrap()
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(c.s.state(), SessionState::AgentRunning);
    // A terminal report (cursor position reply) is not a takeover.
    c.s.write_user(b"\x1b[5;1R").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(c.s.state(), SessionState::AgentRunning);
    c.s.write_user(b"\x03").await.unwrap();
    let r = h.await.unwrap();
    assert_eq!(r.state, CommandState::Interrupted);
    assert_eq!(r.note.as_deref(), Some("user took over"));
}

#[tokio::test(start_paused = true)]
async fn concurrent_agents_get_busy() {
    let c = start(Kind::Bash, auto()).await;
    let s2 = c.s.clone();
    let h = tokio::spawn(async move {
        let ap = TestApprover::approve();
        s2.run_command(&actor(), req("sleep 2"), &ap).await.unwrap()
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    let ap = TestApprover::approve();
    let e = c.s.run_command(&actor(), req("ls"), &ap).await.unwrap_err();
    assert!(matches!(e, Error::Busy(_)));
    let r = h.await.unwrap();
    assert_eq!(
        (r.state, r.output.as_str()),
        (CommandState::Completed, "slept\n")
    );
}

#[tokio::test(start_paused = true)]
async fn approval_mode_by_risk_on_session() {
    use ApprovalMode::*;
    let cmds = [
        ("ls", RiskClass::Read),
        ("touch x", RiskClass::Write),
        ("rm -rf x", RiskClass::Destructive),
    ];
    for mode in [ConfirmAll, ConfirmWrites, Auto] {
        for env in [Environment::Dev, Environment::Prod] {
            let c = start(
                Kind::Bash,
                Policy {
                    mode,
                    env,
                    deny_list: vec![],
                },
            )
            .await;
            for (cmd, risk) in cmds {
                let ap = TestApprover::approve();
                let r = c.s.run_command(&actor(), req(cmd), &ap).await.unwrap();
                assert_eq!(r.state, CommandState::Completed);
                let eff = if mode == Auto && env == Environment::Prod {
                    ConfirmWrites
                } else {
                    mode
                };
                let expect = match eff {
                    ConfirmAll => true,
                    ConfirmWrites => risk != RiskClass::Read,
                    Auto => risk == RiskClass::Destructive,
                };
                assert_eq!(ap.count() == 1, expect, "{mode:?}/{env:?}/{cmd}");
                if expect {
                    let call = ap.calls.lock().unwrap()[0].clone();
                    assert_eq!(call.risk, risk);
                    assert_eq!(call.command, cmd);
                    assert!(call.policy_note.is_some());
                    assert_eq!(call.actor_label, "Test Agent");
                }
            }
        }
    }
}

#[tokio::test(start_paused = true)]
async fn deny_list_denied_and_edited() {
    let c = start(
        Kind::Bash,
        Policy {
            mode: ApprovalMode::Auto,
            env: Environment::Dev,
            deny_list: vec![Regex::new(r"^echo danger").unwrap()],
        },
    )
    .await;
    // Deny-list hit in Auto mode asks; the user denies.
    let ap = TestApprover::new(ApprovalDecision::Deny {
        message: Some("no way".into()),
    });
    let r =
        c.s.run_command(&actor(), req("echo danger zone"), &ap)
            .await
            .unwrap();
    assert_eq!(r.state, CommandState::Denied);
    assert_eq!(r.note.as_deref(), Some("no way"));
    assert!(ap.calls.lock().unwrap()[0]
        .policy_note
        .as_deref()
        .unwrap()
        .starts_with("matches deny-list"));
    assert!(
        !c.sh.written().contains("danger"),
        "denied command never typed"
    );

    // ApproveEdited runs the edited command.
    let ap = TestApprover::new(ApprovalDecision::ApproveEdited {
        command: "echo safe".into(),
    });
    let r =
        c.s.run_command(&actor(), req("echo danger again"), &ap)
            .await
            .unwrap();
    assert_eq!(r.state, CommandState::Completed);
    assert_eq!(r.command, "echo safe");
    assert_eq!(r.output, "safe\n");
    let audit = c.s.audit_log();
    assert!(matches!(audit[0].decision, AuditDecision::Denied { .. }));
    assert_eq!(audit[0].state, CommandState::Denied);
    assert!(matches!(
        &audit[1].decision,
        AuditDecision::ApprovedEdited { original } if original == "echo danger again"
    ));
}

#[tokio::test(start_paused = true)]
async fn disconnect_interrupts() {
    let c = start(Kind::Bash, auto()).await;
    let s2 = c.s.clone();
    let h = tokio::spawn(async move {
        let ap = TestApprover::approve();
        s2.run_command(&actor(), req("sleep 10"), &ap)
            .await
            .unwrap()
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    c.sh.disconnect();
    let r = h.await.unwrap();
    assert_eq!(r.state, CommandState::Interrupted);
    assert_eq!(r.note.as_deref(), Some("connection closed"));
    assert_eq!(c.s.state(), SessionState::Disconnected);
    let ap = TestApprover::approve();
    assert!(c.s.run_command(&actor(), req("ls"), &ap).await.is_err());
}

#[tokio::test(start_paused = true)]
async fn send_keys_interrupts_timed_out_command() {
    let c = start(Kind::Bash, auto()).await;
    let ap = TestApprover::approve();
    let r =
        c.s.run_command(&actor(), req_t("sleep 100", 1), &ap)
            .await
            .unwrap();
    assert_eq!(r.state, CommandState::TimedOut);
    c.s.send_keys(&actor(), &[ControlKey::CtrlC]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        c.s.read_output(r.output_id.unwrap(), 0, 10)
            .unwrap()
            .complete
    );
    let r2 = c.s.run_command(&actor(), req("false"), &ap).await.unwrap();
    assert_eq!(r2.exit_code, Some(1));
    let log = c.s.audit_log();
    assert!(log.iter().any(|e| e.command == "[keys] ctrl_c"));
}

#[tokio::test(start_paused = true)]
async fn scope_and_validation() {
    let c = start(Kind::Bash, auto()).await;
    let ap = TestApprover::approve();
    let outsider = Actor {
        session_scope: Some(vec![]),
        ..actor()
    };
    assert!(matches!(
        c.s.run_command(&outsider, req("ls"), &ap).await,
        Err(Error::Denied(_))
    ));
    assert!(matches!(
        c.s.run_command(&actor(), req("ls\nrm -rf /"), &ap).await,
        Err(Error::Invalid(_))
    ));
}

#[tokio::test(start_paused = true)]
async fn user_typing_during_injection_is_queued() {
    let (sh, rx) = FakeShell::new(Kind::Bash);
    let s = TermSession::spawn(Arc::new(FakeHandle(sh.clone())), rx, TermConfig::default());
    // Wait until the probe has been sent, then type.
    for _ in 0..100 {
        if sh.written().contains("AHTPROBE") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    s.write_user(b"echo typed\r").await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(s.integration(), IntegrationMode::Full);
    let w = sh.written();
    let hook_end = w.find("AhtEnd").unwrap();
    assert!(
        w.rfind("echo typed").unwrap() > hook_end,
        "user input flushed after hook"
    );
    assert!(s.recent_lines(10).iter().any(|l| l == "typed"));
}

/// Output and marks arriving in one big chunk (as ConPTY does): the partial line at `D` must
/// not include the prompt that follows it in the same chunk.
#[tokio::test(start_paused = true)]
async fn whole_chunk_delivery() {
    let (sh, rx) = FakeShell::with_chunk(Kind::Bash, usize::MAX);
    let s = TermSession::spawn(
        Arc::new(FakeHandle(sh.clone())),
        rx,
        TermConfig {
            policy: auto(),
            ..Default::default()
        },
    );
    for _ in 0..100 {
        if s.integration() == IntegrationMode::Full && s.state() == SessionState::Idle {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let ap = TestApprover::approve();
    let r = s
        .run_command(&actor(), req("printf 'tail'"), &ap)
        .await
        .unwrap();
    assert_eq!(r.output, "tail");
    let r = s.run_command(&actor(), req("echo one"), &ap).await.unwrap();
    assert_eq!(
        r.output,
        "one
"
    );
}
