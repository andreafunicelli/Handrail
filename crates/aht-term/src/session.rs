//! `TermSession`: the single control path into one terminal.

use crate::hooks::{self, ShellKind};
use crate::parser::{TrackEvent, Tracker, UiFilter, CLEAR_LINE};
use crate::policy::Policy;
use crate::store::{excerpt, OutputStore};
use aht_core::agent::{ApprovalDecision, ApprovalRequest, RiskClass};
use aht_core::terminal::{
    Actor, CommandResult, CommandState, ControlKey, IntegrationMode, OutputChunk,
    RunCommandRequest, SessionInfo, SessionState, Transport, TransportEvent,
};
use aht_core::{ApprovalId, ConnectionId, Error, OutputId, Result, SessionId};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::time::Instant;

/// Asks the user to approve an agent command (UI approval card).
#[async_trait]
pub trait Approver: Send + Sync {
    async fn request(&self, req: ApprovalRequest) -> ApprovalDecision;
}

/// Session configuration.
#[derive(Clone)]
pub struct TermConfig {
    pub session_id: SessionId,
    pub connection_id: Option<ConnectionId>,
    pub title: String,
    pub policy: Policy,
    /// Limited mode: output quiescence that (with a prompt) ends a command.
    pub quiescence: Duration,
    /// Quiescence at a prompt-looking line before the hidden shell probe is sent.
    pub probe_quiescence: Duration,
    /// Max wait for each hidden exchange (probe, hook) before falling back to Limited mode.
    pub detect_timeout: Duration,
    /// Inject shell integration. When false, the session goes straight to Limited mode.
    pub inject: bool,
    /// Shared output store (one per session if `None`).
    pub output_store: Option<Arc<OutputStore>>,
    /// Bytes of head and of tail returned when output is truncated.
    pub excerpt_bytes: usize,
}

impl Default for TermConfig {
    fn default() -> Self {
        Self {
            session_id: SessionId::new(),
            connection_id: None,
            title: String::new(),
            policy: Policy::default(),
            quiescence: Duration::from_millis(1500),
            probe_quiescence: Duration::from_millis(400),
            detect_timeout: Duration::from_secs(3),
            inject: true,
            output_store: None,
            excerpt_bytes: 4096,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuditDecision {
    /// Policy allowed it without asking.
    AutoApproved,
    Approved,
    ApprovedEdited {
        original: String,
    },
    Denied {
        message: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AuditEntry {
    pub at: DateTime<Utc>,
    pub actor_id: String,
    pub actor_label: String,
    pub command: String,
    pub risk: RiskClass,
    pub decision: AuditDecision,
    pub exit_code: Option<i32>,
    pub state: CommandState,
}

/// Session events for subscribers (UI decorations, status bar, audit view).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEvent {
    StateChanged {
        state: SessionState,
    },
    IntegrationChanged {
        mode: IntegrationMode,
    },
    InfoChanged {
        info: SessionInfo,
    },
    /// An agent typed a command into the terminal (decorate the line).
    AgentMark {
        actor_id: String,
        actor_label: String,
        command: String,
        output_id: OutputId,
    },
    AgentDone {
        output_id: OutputId,
        state: CommandState,
        exit_code: Option<i32>,
    },
    Audit {
        entry: AuditEntry,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Init {
    WaitPrompt,
    Probing {
        deadline: Instant,
    },
    Injecting {
        nonce: String,
        deadline: Instant,
    },
    /// Waiting for quiet before ending a hidden exchange that has no end sentinel.
    Settle,
    Done,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CapStatus {
    Running,
    Finished,
    NeedsUser(String),
    Interrupted(String),
    /// The agent stopped waiting (timeout); output keeps being recorded.
    Detached,
}

struct Capture {
    id: OutputId,
    command: String,
    full: bool,
    saw_c: bool,
    /// Full mode: lines seen before `C` (used only if `C` never arrives).
    pre: Vec<String>,
    first_pending: bool,
    drop_first: bool,
    status: CapStatus,
    exit: Option<i32>,
    done: bool,
}

struct Inner {
    tracker: Tracker,
    filter: UiFilter,
    info: SessionInfo,
    init: Init,
    shell: Option<ShellKind>,
    prev_state: SessionState,
    last_output: Instant,
    started: Instant,
    learned_prompt: Option<String>,
    user_dirty: bool,
    pending_user: Vec<u8>,
    capture: Option<Capture>,
    recent: VecDeque<String>,
    audit: Vec<AuditEntry>,
    policy: Policy,
}

const RECENT_LINES: usize = 2000;

struct Shared {
    cfg: TermConfig,
    transport: Arc<dyn Transport>,
    inner: Mutex<Inner>,
    store: Arc<OutputStore>,
    ui_tx: broadcast::Sender<Vec<u8>>,
    ev_tx: broadcast::Sender<SessionEvent>,
    gen_tx: watch::Sender<u64>,
    agent_lock: tokio::sync::Mutex<()>,
}

/// One live terminal session. Cheap to clone (shared handle).
#[derive(Clone)]
pub struct TermSession {
    s: Arc<Shared>,
}

fn password_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i:[Pp]assword( for [^:]+)?:\s*$)|(?i:passphrase.*:\s*$)|\[sudo\]|Verification code:|(?i:password:\s*$)",
        )
        .unwrap()
    })
}

/// Terminal-generated replies (cursor position, device attributes, focus) are not user input.
fn is_terminal_report(b: &[u8]) -> bool {
    if b == b"\x1b[I" || b == b"\x1b[O" {
        return true;
    }
    b.len() > 2
        && b.starts_with(b"\x1b[")
        && matches!(b.last(), Some(b'R' | b'c' | b'n'))
        && b[2..b.len() - 1]
            .iter()
            .all(|c| c.is_ascii_digit() || matches!(c, b';' | b'?' | b'>'))
}

fn new_nonce() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..16].to_string()
}

impl Inner {
    fn set_state(&mut self, sh: &Shared, st: SessionState) {
        if self.info.state != st {
            self.info.state = st;
            let _ = sh.ev_tx.send(SessionEvent::StateChanged { state: st });
        }
    }

    fn set_integration(&mut self, sh: &Shared, m: IntegrationMode) {
        if self.info.integration != m {
            self.info.integration = m;
            let _ = sh.ev_tx.send(SessionEvent::IntegrationChanged { mode: m });
        }
    }

    fn info_changed(&self, sh: &Shared) {
        let _ = sh.ev_tx.send(SessionEvent::InfoChanged {
            info: self.info.clone(),
        });
    }

    fn hidden(&self) -> bool {
        matches!(
            self.init,
            Init::Probing { .. } | Init::Injecting { .. } | Init::Settle
        )
    }

    fn prompt_matches(&self, line: &str) -> bool {
        let l = line.trim_end();
        if l.is_empty() || l.len() > 300 {
            return false;
        }
        match &self.learned_prompt {
            Some(p) => {
                let p = p.trim_end();
                l == p || (p.chars().last().is_some() && l.ends_with(p.chars().last().unwrap()))
            }
            None => l.ends_with(['$', '#', '>', '%']),
        }
    }

    fn cap_emit(&mut self, sh: &Shared, line: &str, newline: bool) {
        let Some(c) = self.capture.as_mut() else {
            return;
        };
        if c.first_pending {
            c.first_pending = false;
            let cmd = c.command.trim();
            if c.drop_first || (!cmd.is_empty() && line.trim_end().ends_with(cmd)) {
                return;
            }
        }
        sh.store.append(c.id, line);
        if newline {
            sh.store.append(c.id, "\n");
        }
    }

    fn cap_finish(&mut self, sh: &Shared, exit: Option<i32>) {
        let Some(c) = self.capture.as_mut().filter(|c| !c.done) else {
            return;
        };
        c.done = true;
        c.exit = exit;
        if c.status == CapStatus::Running {
            c.status = CapStatus::Finished;
        }
        sh.store.complete(c.id);
    }

    fn active_capture(&self) -> bool {
        self.capture.as_ref().is_some_and(|c| !c.done)
    }

    fn on_line(&mut self, sh: &Shared, line: String, writes: &mut Vec<Vec<u8>>) {
        if let Init::Probing { .. } = self.init {
            if let Some(kind) = hooks::parse_probe(&line) {
                self.shell = Some(kind);
                self.info.shell = Some(kind.name().to_string());
                let nonce = new_nonce();
                match hooks::hook_for(kind, &nonce) {
                    Some(hook) => {
                        self.filter.hide_until(Some(nonce.clone()));
                        self.init = Init::Injecting {
                            nonce,
                            deadline: Instant::now() + sh.cfg.detect_timeout,
                        };
                        let mut w = hook.into_bytes();
                        w.push(b'\r');
                        writes.push(w);
                    }
                    None => self.init = Init::Settle,
                }
            }
            return;
        }
        if self.hidden() {
            return;
        }
        if self.active_capture() {
            let c = self.capture.as_mut().unwrap();
            if c.full && !c.saw_c {
                c.pre.push(line.clone());
            } else {
                self.cap_emit(sh, &line, true);
            }
        }
        if self.recent.len() >= RECENT_LINES {
            self.recent.pop_front();
        }
        self.recent.push_back(line);
    }

    fn handle(&mut self, sh: &Shared, ev: TrackEvent, writes: &mut Vec<Vec<u8>>) {
        match ev {
            TrackEvent::Line(l) => self.on_line(sh, l, writes),
            TrackEvent::PromptStart | TrackEvent::PromptEnd => {
                if self.init == Init::WaitPrompt {
                    // The shell already has OSC 133 integration (e.g. fish 4, user's own rc).
                    self.init = Init::Done;
                    self.set_integration(sh, IntegrationMode::Full);
                }
                if !self.hidden() && !self.tracker.in_alt_screen() {
                    if self.info.integration == IntegrationMode::Full {
                        if let Some(c) = &self.capture {
                            if !c.done && c.full && c.saw_c {
                                // D was lost (e.g. Ctrl-C in some shells): finish without code.
                                self.cap_finish(sh, None);
                            }
                        }
                    }
                    self.set_state(sh, SessionState::Idle);
                }
            }
            TrackEvent::CommandStart => {
                if self.hidden() {
                    return;
                }
                let line_nonempty = !self.tracker.current_line().trim().is_empty();
                let agent = match self.capture.as_mut() {
                    Some(c) if !c.done && c.full && !c.saw_c => {
                        c.saw_c = true;
                        c.pre.clear();
                        c.first_pending = true;
                        c.drop_first = line_nonempty;
                        c.status == CapStatus::Running
                    }
                    _ => false,
                };
                self.set_state(
                    sh,
                    if agent {
                        SessionState::AgentRunning
                    } else {
                        SessionState::Busy
                    },
                );
            }
            TrackEvent::CommandEnd(code) => {
                if self.hidden() {
                    return;
                }
                if let Some(c) = self.capture.as_mut() {
                    if !c.done && c.full {
                        if !c.saw_c {
                            let pre = std::mem::take(&mut c.pre);
                            c.first_pending = true;
                            c.drop_first = true;
                            for l in pre {
                                self.cap_emit(sh, &l, true);
                            }
                        }
                        let partial = self.tracker.current_line();
                        if !partial.trim().is_empty() {
                            self.cap_emit(sh, partial.trim_end(), false);
                        }
                        self.cap_finish(sh, code);
                    }
                }
                if self.info.state == SessionState::AgentRunning {
                    self.set_state(sh, SessionState::Busy);
                }
            }
            TrackEvent::Cwd(p) => {
                if self.info.cwd.as_deref() != Some(&p) {
                    self.info.cwd = Some(p);
                    self.info_changed(sh);
                }
            }
            TrackEvent::AltScreen(on) => {
                if on {
                    self.prev_state = self.info.state;
                    self.set_state(sh, SessionState::FullScreen);
                    if let Some(c) = self.capture.as_mut() {
                        if !c.done && c.status == CapStatus::Running {
                            c.status = CapStatus::NeedsUser(
                                "a full-screen program started; the user must interact with it"
                                    .into(),
                            );
                        }
                    }
                } else {
                    let prev = match self.prev_state {
                        SessionState::FullScreen => SessionState::Busy,
                        s => s,
                    };
                    self.set_state(sh, prev);
                }
            }
            TrackEvent::SentinelEnd(n) => {
                if let Init::Injecting { nonce, .. } = &self.init {
                    if *nonce == n {
                        self.filter.release();
                        self.init = Init::Done;
                        self.set_integration(sh, IntegrationMode::Full);
                        self.set_state(sh, SessionState::Idle);
                        self.info_changed(sh);
                        if !self.pending_user.is_empty() {
                            writes.push(std::mem::take(&mut self.pending_user));
                        }
                    }
                }
            }
            TrackEvent::Kv(k, v) => {
                let v = (!v.is_empty()).then_some(v);
                match k.as_str() {
                    "os" => self.info.os = v,
                    "user" => self.info.user = v,
                    "host" => self.info.host = v,
                    "shell" => self.info.shell = v,
                    _ => {}
                }
            }
            TrackEvent::SentinelBegin(_) => {}
        }
    }

    fn release_hidden(&mut self, sh: &Shared, writes: &mut Vec<Vec<u8>>) {
        self.filter.release();
        let cur = self.tracker.current_line();
        let mut redraw = CLEAR_LINE.to_vec();
        redraw.extend_from_slice(cur.as_bytes());
        let _ = sh.ui_tx.send(redraw);
        if !cur.trim().is_empty() {
            self.learned_prompt = Some(cur);
        }
        self.init = Init::Done;
        if self.info.integration != IntegrationMode::Full {
            self.set_integration(sh, IntegrationMode::Limited);
        }
        self.set_state(sh, SessionState::Idle);
        self.info_changed(sh);
        if !self.pending_user.is_empty() {
            writes.push(std::mem::take(&mut self.pending_user));
        }
    }

    fn on_tick(&mut self, sh: &Shared, writes: &mut Vec<Vec<u8>>) {
        let now = Instant::now();
        let quiet = now.saturating_duration_since(self.last_output);
        match self.init.clone() {
            Init::WaitPrompt => {
                let cur = self.tracker.current_line();
                let at_prompt = !cur.trim().is_empty()
                    && quiet >= sh.cfg.probe_quiescence
                    && !self.user_dirty
                    && !self.tracker.in_alt_screen();
                let give_up = now.saturating_duration_since(self.started)
                    > sh.cfg.detect_timeout * 3
                    && quiet >= sh.cfg.probe_quiescence
                    && !self.tracker.in_alt_screen();
                if at_prompt && sh.cfg.inject {
                    self.learned_prompt = Some(cur);
                    self.filter.hide_until(None);
                    self.init = Init::Probing {
                        deadline: now + sh.cfg.detect_timeout,
                    };
                    let mut w = hooks::PROBE.as_bytes().to_vec();
                    w.push(b'\r');
                    writes.push(w);
                } else if at_prompt || give_up {
                    if !cur.trim().is_empty() {
                        self.learned_prompt = Some(cur);
                    }
                    self.init = Init::Done;
                    self.set_integration(sh, IntegrationMode::Limited);
                    self.set_state(sh, SessionState::Idle);
                }
            }
            Init::Probing { deadline } | Init::Injecting { deadline, .. } => {
                if now >= deadline {
                    self.init = Init::Settle;
                }
            }
            Init::Settle => {
                if quiet >= Duration::from_millis(200) {
                    self.release_hidden(sh, writes);
                }
            }
            Init::Done => {
                if self.info.integration == IntegrationMode::Limited
                    && quiet >= sh.cfg.quiescence
                    && !self.tracker.in_alt_screen()
                {
                    let cur = self.tracker.current_line();
                    if self.prompt_matches(&cur) {
                        if let Some(c) = &self.capture {
                            if !c.done && !c.first_pending {
                                self.cap_finish(sh, None);
                            }
                        }
                        if !self.active_capture() {
                            self.set_state(sh, SessionState::Idle);
                        }
                    }
                }
            }
        }
    }

    fn after_chunk(&mut self) {
        if let Some(c) = self.capture.as_mut() {
            if !c.done && c.status == CapStatus::Running {
                let cur = self.tracker.current_line();
                if password_re().is_match(cur.trim_end()) || password_re().is_match(&cur) {
                    c.status = CapStatus::NeedsUser(format!(
                        "the command is asking for input ({}); the user must type it",
                        cur.trim()
                    ));
                }
            }
        }
    }
}

impl TermSession {
    /// Start a session over `transport`; spawns the output reader task (requires a Tokio runtime).
    pub fn spawn(
        transport: Arc<dyn Transport>,
        events: mpsc::Receiver<TransportEvent>,
        cfg: TermConfig,
    ) -> Self {
        let now = Instant::now();
        let info = SessionInfo {
            session_id: cfg.session_id,
            connection_id: cfg.connection_id,
            title: cfg.title.clone(),
            env: cfg.policy.env,
            os: None,
            shell: None,
            cwd: None,
            user: None,
            host: None,
            integration: IntegrationMode::Unknown,
            state: SessionState::Connecting,
            approval_mode: cfg.policy.mode,
        };
        let inner = Inner {
            tracker: Tracker::new(),
            filter: UiFilter::default(),
            info,
            init: Init::WaitPrompt,
            shell: None,
            prev_state: SessionState::Busy,
            last_output: now,
            started: now,
            learned_prompt: None,
            user_dirty: false,
            pending_user: Vec::new(),
            capture: None,
            recent: VecDeque::new(),
            audit: Vec::new(),
            policy: cfg.policy.clone(),
        };
        let store = cfg.output_store.clone().unwrap_or_default();
        let shared = Arc::new(Shared {
            transport,
            inner: Mutex::new(inner),
            store,
            ui_tx: broadcast::channel(4096).0,
            ev_tx: broadcast::channel(1024).0,
            gen_tx: watch::channel(0).0,
            agent_lock: tokio::sync::Mutex::new(()),
            cfg,
        });
        tokio::spawn(reader_task(shared.clone(), events));
        Self { s: shared }
    }

    pub fn id(&self) -> SessionId {
        self.s.cfg.session_id
    }

    pub fn info(&self) -> SessionInfo {
        self.s.inner.lock().unwrap().info.clone()
    }

    pub fn state(&self) -> SessionState {
        self.s.inner.lock().unwrap().info.state
    }

    pub fn integration(&self) -> IntegrationMode {
        self.s.inner.lock().unwrap().info.integration
    }

    /// Raw output for xterm.js (private sentinels stripped, hidden injections suppressed).
    pub fn subscribe_ui(&self) -> broadcast::Receiver<Vec<u8>> {
        self.s.ui_tx.subscribe()
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<SessionEvent> {
        self.s.ev_tx.subscribe()
    }

    pub fn audit_log(&self) -> Vec<AuditEntry> {
        self.s.inner.lock().unwrap().audit.clone()
    }

    /// Last `n` ANSI-stripped output lines.
    pub fn recent_lines(&self, n: usize) -> Vec<String> {
        let g = self.s.inner.lock().unwrap();
        let skip = g.recent.len().saturating_sub(n);
        g.recent.iter().skip(skip).cloned().collect()
    }

    pub fn set_policy(&self, policy: Policy) {
        let mut g = self.s.inner.lock().unwrap();
        g.info.approval_mode = policy.mode;
        g.info.env = policy.env;
        g.policy = policy;
    }

    pub fn policy(&self) -> Policy {
        self.s.inner.lock().unwrap().policy.clone()
    }

    pub fn output_store(&self) -> Arc<OutputStore> {
        self.s.store.clone()
    }

    /// User keystrokes from the terminal UI.
    pub async fn write_user(&self, data: &[u8]) -> Result<()> {
        {
            let mut g = self.s.inner.lock().unwrap();
            if g.hidden() {
                // Hold keystrokes until the hidden injection is done so they don't mix with it.
                g.pending_user.extend_from_slice(data);
                return Ok(());
            }
            let report = is_terminal_report(data);
            if !report {
                // "Dirty" = the user has a partially typed line we must not inject into.
                g.user_dirty = !data.iter().any(|b| matches!(b, b'\r' | b'\n' | 0x03));
                let mut took_over = false;
                if let Some(c) = g.capture.as_mut() {
                    if !c.done && c.status == CapStatus::Running {
                        c.status = CapStatus::Interrupted("user took over".into());
                        took_over = true;
                    }
                }
                let sh = &*self.s;
                let limited_enter = g.info.integration != IntegrationMode::Full
                    && g.init == Init::Done
                    && g.info.state == SessionState::Idle
                    && data.contains(&b'\r');
                if took_over || limited_enter {
                    g.set_state(sh, SessionState::Busy);
                }
                self.s.gen_tx.send_modify(|v| *v += 1);
            }
        }
        self.s.transport.write(data).await
    }

    pub async fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.s.transport.resize(cols, rows).await
    }

    pub async fn close(&self) -> Result<()> {
        self.s.transport.close().await
    }

    pub fn read_output(
        &self,
        output: OutputId,
        offset: usize,
        limit: usize,
    ) -> Result<OutputChunk> {
        self.s.store.read(output, offset, limit)
    }

    /// Send control keys on behalf of an agent (only [`ControlKey`] bytes can be sent).
    pub async fn send_keys(&self, actor: &Actor, keys: &[ControlKey]) -> Result<()> {
        if !actor.may_access(self.id()) {
            return Err(Error::Denied("session outside the actor's scope".into()));
        }
        let bytes: Vec<u8> = keys
            .iter()
            .flat_map(|k| k.bytes().iter().copied())
            .collect();
        self.s.transport.write(&bytes).await?;
        let names: Vec<String> = keys
            .iter()
            .map(|k| {
                serde_json::to_value(k)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_default()
            })
            .collect();
        self.audit(AuditEntry {
            at: Utc::now(),
            actor_id: actor.id.clone(),
            actor_label: actor.label.clone(),
            command: format!("[keys] {}", names.join(" ")),
            risk: RiskClass::Write,
            decision: AuditDecision::AutoApproved,
            exit_code: None,
            state: CommandState::Completed,
        });
        Ok(())
    }

    fn audit(&self, entry: AuditEntry) {
        self.s.inner.lock().unwrap().audit.push(entry.clone());
        let _ = self.s.ev_tx.send(SessionEvent::Audit { entry });
    }

    /// Wait until `cond` holds or `deadline` passes. Returns whether it held.
    async fn wait_until<F: FnMut(&Inner) -> bool>(&self, deadline: Instant, mut cond: F) -> bool {
        let mut rx = self.s.gen_tx.subscribe();
        loop {
            if cond(&self.s.inner.lock().unwrap()) {
                return true;
            }
            match tokio::time::timeout_at(deadline, rx.changed()).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => return cond(&self.s.inner.lock().unwrap()),
                Err(_) => return cond(&self.s.inner.lock().unwrap()),
            }
        }
    }

    /// Run an agent command in the live terminal, subject to the approval policy.
    pub async fn run_command(
        &self,
        actor: &Actor,
        req: RunCommandRequest,
        approver: &dyn Approver,
    ) -> Result<CommandResult> {
        if !actor.may_access(self.id()) {
            return Err(Error::Denied("session outside the actor's scope".into()));
        }
        let original = req.command.trim().to_string();
        validate_command(&original)?;
        let _guard = self.s.agent_lock.try_lock().map_err(|_| {
            Error::Busy("another agent command is already running in this session".into())
        })?;
        let timeout = Duration::from_secs(req.timeout_s.unwrap_or(60).clamp(1, 600));
        let started = Instant::now();

        let policy = self.policy();
        let verdict = policy.evaluate(&original);
        let (command, decision) = if verdict.needs_approval {
            let ar = ApprovalRequest {
                approval_id: ApprovalId::new(),
                session_id: self.id(),
                actor_label: actor.label.clone(),
                command: original.clone(),
                reason: req.reason.clone(),
                risk: verdict.risk,
                policy_note: verdict.note.clone(),
            };
            match approver.request(ar).await {
                ApprovalDecision::Approve => (original.clone(), AuditDecision::Approved),
                ApprovalDecision::ApproveEdited { command } => {
                    let command = command.trim().to_string();
                    validate_command(&command)?;
                    (
                        command,
                        AuditDecision::ApprovedEdited {
                            original: original.clone(),
                        },
                    )
                }
                ApprovalDecision::Deny { message } => {
                    self.audit(AuditEntry {
                        at: Utc::now(),
                        actor_id: actor.id.clone(),
                        actor_label: actor.label.clone(),
                        command: original.clone(),
                        risk: verdict.risk,
                        decision: AuditDecision::Denied {
                            message: message.clone(),
                        },
                        exit_code: None,
                        state: CommandState::Denied,
                    });
                    return Ok(CommandResult {
                        state: CommandState::Denied,
                        exit_code: None,
                        output: String::new(),
                        truncated: false,
                        output_id: None,
                        duration_ms: started.elapsed().as_millis() as u64,
                        command: original,
                        note: Some(message.unwrap_or_else(|| "the user denied the command".into())),
                    });
                }
            }
        } else {
            (original.clone(), AuditDecision::AutoApproved)
        };

        // Wait for the shell to be at a prompt.
        let deadline = Instant::now() + timeout;
        self.wait_until(deadline, |g| {
            g.info.state == SessionState::Disconnected
                || g.info.state == SessionState::FullScreen
                || (g.init == Init::Done && g.info.state == SessionState::Idle)
        })
        .await;
        let output_id = {
            let mut g = self.s.inner.lock().unwrap();
            match g.info.state {
                SessionState::Disconnected => {
                    return Err(Error::Network("session is disconnected".into()))
                }
                SessionState::FullScreen => {
                    return Err(Error::Busy(
                        "a full-screen program is active in the terminal".into(),
                    ))
                }
                SessionState::Idle if g.init == Init::Done => {}
                _ => return Err(Error::Busy("the terminal is not at a shell prompt".into())),
            }
            let id = self.s.store.create();
            let full = g.info.integration == IntegrationMode::Full;
            g.capture = Some(Capture {
                id,
                command: command.clone(),
                full,
                saw_c: false,
                pre: Vec::new(),
                first_pending: true,
                drop_first: !full,
                status: CapStatus::Running,
                exit: None,
                done: false,
            });
            g.user_dirty = false;
            g.set_state(&self.s, SessionState::AgentRunning);
            let _ = self.s.ev_tx.send(SessionEvent::AgentMark {
                actor_id: actor.id.clone(),
                actor_label: actor.label.clone(),
                command: command.clone(),
                output_id: id,
            });
            id
        };
        let mut line = command.clone().into_bytes();
        line.push(b'\r');
        if let Err(e) = self.s.transport.write(&line).await {
            let mut g = self.s.inner.lock().unwrap();
            g.cap_finish(&self.s, None);
            g.set_state(&self.s, SessionState::Idle);
            return Err(e);
        }

        let finished = self
            .wait_until(started + timeout, |g| {
                g.capture
                    .as_ref()
                    .is_none_or(|c| c.id != output_id || c.status != CapStatus::Running)
            })
            .await;

        let (state, exit_code, note, partial) = {
            let mut g = self.s.inner.lock().unwrap();
            let partial = g.tracker.current_line();
            let c = g.capture.as_mut().filter(|c| c.id == output_id);
            let (state, exit, note, done) = match c {
                None => (
                    CommandState::Interrupted,
                    None,
                    Some("capture lost".into()),
                    true,
                ),
                Some(c) => {
                    if !finished && c.status == CapStatus::Running {
                        c.status = CapStatus::Detached;
                    }
                    match &c.status {
                        CapStatus::Finished => (CommandState::Completed, c.exit, None, true),
                        CapStatus::NeedsUser(n) => {
                            (CommandState::NeedsUser, None, Some(n.clone()), c.done)
                        }
                        CapStatus::Interrupted(n) => {
                            (CommandState::Interrupted, None, Some(n.clone()), c.done)
                        }
                        CapStatus::Detached | CapStatus::Running => (
                            CommandState::TimedOut,
                            None,
                            Some(format!(
                                "still running after {}s; poll read_output or send ctrl_c",
                                timeout.as_secs()
                            )),
                            c.done,
                        ),
                    }
                }
            };
            if matches!(state, CommandState::NeedsUser)
                && g.info.state == SessionState::AgentRunning
            {
                g.set_state(&self.s, SessionState::Busy);
            }
            (state, exit, note, (!done).then_some(partial))
        };

        let (mut text, _) = self.s.store.get(output_id).unwrap_or_default();
        if let Some(p) = partial {
            if !p.trim().is_empty() {
                text.push_str(p.trim_end());
            }
        }
        let (output, truncated) =
            excerpt(&text, self.s.cfg.excerpt_bytes, self.s.cfg.excerpt_bytes);
        self.audit(AuditEntry {
            at: Utc::now(),
            actor_id: actor.id.clone(),
            actor_label: actor.label.clone(),
            command: command.clone(),
            risk: verdict.risk,
            decision,
            exit_code,
            state,
        });
        let _ = self.s.ev_tx.send(SessionEvent::AgentDone {
            output_id,
            state,
            exit_code,
        });
        Ok(CommandResult {
            state,
            exit_code,
            output,
            truncated,
            output_id: Some(output_id),
            duration_ms: started.elapsed().as_millis() as u64,
            command,
            note,
        })
    }
}

fn validate_command(cmd: &str) -> Result<()> {
    if cmd.is_empty() {
        return Err(Error::Invalid("empty command".into()));
    }
    if cmd.chars().any(|c| c.is_control()) {
        return Err(Error::Invalid(
            "command must be a single line without control characters".into(),
        ));
    }
    Ok(())
}

async fn reader_task(sh: Arc<Shared>, mut rx: mpsc::Receiver<TransportEvent>) {
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        let mut writes = Vec::new();
        let mut closed = false;
        tokio::select! {
            ev = rx.recv() => match ev {
                Some(TransportEvent::Data(d)) => {
                    let mut g = sh.inner.lock().unwrap();
                    g.last_output = Instant::now();
                    let ui = g.filter.filter(&d);
                    if !ui.is_empty() {
                        let _ = sh.ui_tx.send(ui);
                    }
                    let events = g.tracker.feed(&d);
                    for ev in events {
                        g.handle(&sh, ev, &mut writes);
                    }
                    g.after_chunk();
                }
                Some(TransportEvent::Closed { .. }) | Some(TransportEvent::Error(_)) | None => {
                    closed = true;
                }
            },
            _ = tick.tick() => {
                sh.inner.lock().unwrap().on_tick(&sh, &mut writes);
            }
        }
        if closed {
            let mut g = sh.inner.lock().unwrap();
            if let Some(c) = g.capture.as_mut() {
                if !c.done && c.status == CapStatus::Running {
                    c.status = CapStatus::Interrupted("connection closed".into());
                }
            }
            g.cap_finish(&sh, None);
            g.filter.release();
            g.init = Init::Done;
            g.set_state(&sh, SessionState::Disconnected);
            drop(g);
            sh.gen_tx.send_modify(|v| *v += 1);
            return;
        }
        sh.gen_tx.send_modify(|v| *v += 1);
        for w in writes {
            if let Err(e) = sh.transport.write(&w).await {
                tracing::warn!("aht-term: transport write failed: {e}");
            }
        }
    }
}
