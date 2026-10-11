//! Terminal sessions, approvals, agents and the MCP bridge.
//!
//! Everything that acts on a terminal goes through [`TermSession`] (aht-term), which enforces
//! the approval policy. The built-in agent and every MCP-connected harness use the same
//! [`Tools`] implementation of `TerminalTools`, so both see identical behavior.

use crate::cmd_vault::secret_value;
use crate::state::{err, AppState};
use aht_core::agent::{AgentEvent, ApprovalDecision, ApprovalRequest};
use aht_core::model::{
    ApprovalMode, Brain, Connection, ConnectionKind, HarnessDisplay, HarnessKind, HarnessProfile,
    SshAuth,
};
use aht_core::terminal::{
    Actor, CommandResult, ConnectionSummary, ControlKey, FileEditOutcome, FileEditRequest,
    FileEditResult, OutputChunk, RunCommandRequest, SessionInfo, TerminalTools, Transport,
    TransportEvent,
};
use aht_core::{ApprovalId, ConnectionId, Error, OutputId, Result, SessionId};
use aht_harness::{HeadlessRun, LaunchContext, LaunchMode, LaunchSpec};
use aht_mcp::{McpOptions, McpServer, McpToken};
use aht_ssh::{
    AuthMaterial, HostKeyPrompt, JumpHop, KeyboardInteractiveHandler, KiPrompt, KnownHosts,
    PromptingVerifier, SshParams,
};
use aht_term::{Approver, Policy, SessionEvent, TermConfig, TermSession};
use async_trait::async_trait;
use base64::Engine;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{broadcast, mpsc, oneshot, Mutex, OnceCell, RwLock};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

struct SessionEntry {
    term: Arc<TermSession>,
    /// The SSH connection behind an SSH session (for SFTP on the same login).
    ssh: Option<Arc<aht_ssh::SshConnection>>,
    /// MCP tokens bound to harnesses launched from (or in) this session; revoked on close.
    tokens: Vec<McpToken>,
    temp_files: Vec<PathBuf>,
    /// The connection's port forwards; dropped (stopped) with the entry.
    forwards: Vec<aht_ssh::Forward>,
    tunnels: Vec<crate::tunnels::TunnelStatus>,
}

enum RunKind {
    BuiltIn(CancellationToken),
    Harness(Arc<HeadlessRun>),
}

struct Run {
    session_id: SessionId,
    kind: RunKind,
}

struct HarnessChat {
    run: Arc<HeadlessRun>,
    token: McpToken,
    /// Run id the UI currently tracks for this chat (follow-ups get a new one).
    run_id: Arc<std::sync::Mutex<String>>,
}

/// Answers to a login prompt; `None` when the user cancelled.
type LoginAnswers = Option<Vec<Zeroizing<String>>>;

type PendingApprovals = HashMap<ApprovalId, (SessionId, oneshot::Sender<ApprovalDecision>)>;
/// File edits an agent proposed, by proposal id: the file's path and where the answer goes.
type PendingEdits = HashMap<String, (String, oneshot::Sender<FileEditResult>)>;

#[derive(Default)]
pub struct TerminalHub {
    app: std::sync::OnceLock<AppHandle>,
    sessions: RwLock<HashMap<SessionId, SessionEntry>>,
    approvals: std::sync::Mutex<PendingApprovals>,
    edits: std::sync::Mutex<PendingEdits>,
    /// Sessions an agent opened itself (via `open_session`), added to its scope.
    grants: std::sync::Mutex<HashMap<String, HashSet<SessionId>>>,
    host_keys: Mutex<HashMap<String, oneshot::Sender<bool>>>,
    /// Login prompts waiting for the user's answers (`None` = cancelled).
    auth_prompts: Mutex<HashMap<String, oneshot::Sender<LoginAnswers>>>,
    mcp: OnceCell<McpServer>,
    agents: Mutex<HashMap<String, Arc<Mutex<aht_agent::Agent>>>>,
    harness_chats: Mutex<HashMap<(SessionId, String), HarnessChat>>,
    runs: Mutex<HashMap<String, Run>>,
    /// SFTP sessions opened on SSH sessions' connections (file browser), by session and
    /// whether the server runs as root (through sudo).
    pub(crate) sftp: Mutex<HashMap<(SessionId, bool), Arc<aht_ssh::Sftp>>>,
    /// Sessions whose file browser is in root mode right now. The root client stays cached
    /// after the toggle goes off (root editors still save through it), so agent proposals ask
    /// this instead of whether a root client exists.
    pub(crate) sftp_root_active: std::sync::Mutex<HashSet<SessionId>>,
}

impl TerminalHub {
    pub fn attach(&self, app: AppHandle) {
        let _ = self.app.set(app);
    }

    fn app(&self) -> &AppHandle {
        self.app.get().expect("TerminalHub attached at setup")
    }

    pub(crate) async fn session(&self, id: SessionId) -> Result<Arc<TermSession>> {
        self.sessions
            .read()
            .await
            .get(&id)
            .map(|e| e.term.clone())
            .ok_or_else(|| Error::NotFound(format!("session {id}")))
    }

    /// The SSH connection of an SSH session.
    pub async fn ssh_connection(&self, id: SessionId) -> Result<Arc<aht_ssh::SshConnection>> {
        self.sessions
            .read()
            .await
            .get(&id)
            .ok_or_else(|| Error::NotFound(format!("session {id}")))?
            .ssh
            .clone()
            .ok_or_else(|| Error::Invalid("files are only available for SSH sessions".into()))
    }

    /// How the port forwards of a session are doing.
    pub(crate) async fn tunnels(&self, id: SessionId) -> Result<Vec<crate::tunnels::TunnelStatus>> {
        self.sessions
            .read()
            .await
            .get(&id)
            .map(|e| e.tunnels.clone())
            .ok_or_else(|| Error::NotFound(format!("session {id}")))
    }

    fn revoke(&self, token: &McpToken) {
        if let Some(mcp) = self.mcp.get() {
            mcp.revoke(token);
        }
    }

    /// Vault locked: stop every agent and forget cached clients holding decrypted keys.
    pub async fn on_lock(&self) {
        let runs: Vec<Run> = self.runs.lock().await.drain().map(|(_, r)| r).collect();
        for r in runs {
            match r.kind {
                RunKind::BuiltIn(c) => c.cancel(),
                RunKind::Harness(h) => h.cancel().await,
            }
        }
        let chats: Vec<HarnessChat> = self
            .harness_chats
            .lock()
            .await
            .drain()
            .map(|(_, c)| c)
            .collect();
        for c in chats {
            c.run.cancel().await;
            self.revoke(&c.token);
        }
        self.agents.lock().await.clear();
        self.grants.lock().unwrap().clear();
        // Dropping the senders denies the commands still waiting for approval.
        let pending: Vec<_> = self.approvals.lock().unwrap().drain().collect();
        drop(pending);
    }

    async fn mcp(&self) -> Result<&McpServer> {
        let app = self.app().clone();
        self.mcp
            .get_or_try_init(|| async move {
                McpServer::start(Arc::new(Tools { app }), McpOptions::default()).await
            })
            .await
    }
}

fn hub(app: &AppHandle) -> &TerminalHub {
    &app.state::<AppState>().inner().terminal
}

// ---------- events ----------

#[derive(Clone, Serialize)]
struct DataEvent {
    session_id: SessionId,
    data_b64: String,
}

#[derive(Clone, Serialize)]
struct AgentMarkEvent {
    session_id: SessionId,
    command: String,
    actor_label: String,
    output_id: OutputId,
}

#[derive(Clone, Serialize)]
struct AgentRunEvent {
    run_id: String,
    session_id: SessionId,
    event: AgentEvent,
}

#[derive(Clone, Serialize)]
struct HostKeyEvent {
    request_id: String,
    host: String,
    port: u16,
    fingerprint: String,
    old_fingerprint: Option<String>,
    kind: &'static str,
}

/// The server (or the password method) asks the user something while logging in. The UI shows it
/// in the pane identified by `prompt_ref` and answers with [`auth_prompt_answer`].
#[derive(Clone, Serialize)]
struct AuthPromptEvent {
    request_id: String,
    prompt_ref: String,
    name: String,
    instructions: String,
    prompts: Vec<AuthPromptField>,
}

#[derive(Clone, Serialize)]
struct AuthPromptField {
    prompt: String,
    echo: bool,
}

/// Register a running session and pump its output/events to the UI.
async fn register(
    app: &AppHandle,
    transport: Arc<dyn Transport>,
    events: mpsc::Receiver<TransportEvent>,
    cfg: TermConfig,
    tokens: Vec<McpToken>,
    temp_files: Vec<PathBuf>,
) -> SessionInfo {
    let term = Arc::new(TermSession::spawn(transport, events, cfg));
    let id = term.id();
    let mut ui = term.subscribe_ui();
    let mut ev = term.subscribe_events();
    let a = app.clone();
    tokio::spawn(async move {
        loop {
            match ui.recv().await {
                Ok(bytes) => {
                    let _ = a.emit(
                        "session://data",
                        DataEvent {
                            session_id: id,
                            data_b64: B64.encode(bytes),
                        },
                    );
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    let a = app.clone();
    let t = term.clone();
    tokio::spawn(async move {
        loop {
            match ev.recv().await {
                Ok(SessionEvent::AgentMark {
                    actor_label,
                    command,
                    output_id,
                    ..
                }) => {
                    let _ = a.emit(
                        "session://agent_mark",
                        AgentMarkEvent {
                            session_id: id,
                            command,
                            actor_label,
                            output_id,
                        },
                    );
                }
                Ok(_) => {
                    let _ = a.emit("session://state", t.info());
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    let info = term.info();
    hub(app).sessions.write().await.insert(
        id,
        SessionEntry {
            term,
            ssh: None,
            tokens,
            temp_files,
            forwards: Vec::new(),
            tunnels: Vec::new(),
        },
    );
    info
}

// ---------- approvals & host keys ----------

struct UiApprover {
    app: AppHandle,
}

/// Removes a pending approval when its wait ends for any reason (decision, timeout, cancel).
struct PendingGuard<'a> {
    app: &'a AppHandle,
    id: ApprovalId,
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        let removed = hub(self.app).approvals.lock().unwrap().remove(&self.id);
        if removed.is_some() {
            let _ = self.app.emit("approval://resolved", self.id);
        }
    }
}

const APPROVAL_TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[async_trait]
impl Approver for UiApprover {
    async fn request(&self, req: ApprovalRequest) -> ApprovalDecision {
        let (tx, rx) = oneshot::channel();
        let id = req.approval_id;
        hub(&self.app)
            .approvals
            .lock()
            .unwrap()
            .insert(id, (req.session_id, tx));
        let _guard = PendingGuard { app: &self.app, id };
        let _ = self.app.emit("approval://requested", req);
        match tokio::time::timeout(APPROVAL_TIMEOUT, rx).await {
            Ok(Ok(decision)) => decision,
            Ok(Err(_)) => ApprovalDecision::Deny {
                message: Some("cancelled".into()),
            },
            Err(_) => ApprovalDecision::Deny {
                message: Some("no answer within 15 minutes".into()),
            },
        }
    }
}

struct UiHostKeyPrompt {
    app: AppHandle,
}

impl UiHostKeyPrompt {
    async fn ask(&self, ev: HostKeyEvent) -> bool {
        let (tx, rx) = oneshot::channel();
        hub(&self.app)
            .host_keys
            .lock()
            .await
            .insert(ev.request_id.clone(), tx);
        let _ = self.app.emit("session://host_key", ev);
        tokio::time::timeout(Duration::from_secs(300), rx)
            .await
            .ok()
            .and_then(|r| r.ok())
            .unwrap_or(false)
    }
}

/// Login prompts for connections without saved credentials, answered in the connecting pane.
struct UiAuthPrompt {
    app: AppHandle,
    prompt_ref: String,
}

#[async_trait]
impl KeyboardInteractiveHandler for UiAuthPrompt {
    async fn respond(
        &self,
        name: &str,
        instructions: &str,
        prompts: &[KiPrompt],
    ) -> Option<Vec<Zeroizing<String>>> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        hub(&self.app)
            .auth_prompts
            .lock()
            .await
            .insert(request_id.clone(), tx);
        let _ = self.app.emit(
            "session://auth_prompt",
            AuthPromptEvent {
                request_id: request_id.clone(),
                prompt_ref: self.prompt_ref.clone(),
                name: name.into(),
                instructions: instructions.into(),
                prompts: prompts
                    .iter()
                    .map(|p| AuthPromptField {
                        prompt: p.prompt.clone(),
                        echo: p.echo,
                    })
                    .collect(),
            },
        );
        let answer = tokio::time::timeout(Duration::from_secs(300), rx)
            .await
            .ok()
            .and_then(|r| r.ok())
            .flatten();
        hub(&self.app).auth_prompts.lock().await.remove(&request_id);
        answer
    }
}

#[tauri::command(rename_all = "snake_case")]
pub async fn auth_prompt_answer(
    state: tauri::State<'_, AppState>,
    request_id: String,
    answers: Option<Vec<String>>,
) -> Result<()> {
    if let Some(tx) = state.terminal.auth_prompts.lock().await.remove(&request_id) {
        let _ = tx.send(answers.map(|a| a.into_iter().map(Zeroizing::new).collect()));
    }
    Ok(())
}

#[async_trait]
impl HostKeyPrompt for UiHostKeyPrompt {
    async fn confirm_new(&self, host: &str, port: u16, fingerprint: &str) -> bool {
        self.ask(HostKeyEvent {
            request_id: uuid::Uuid::new_v4().to_string(),
            host: host.into(),
            port,
            fingerprint: fingerprint.into(),
            old_fingerprint: None,
            kind: "new",
        })
        .await
    }

    async fn confirm_replace(&self, host: &str, port: u16, old: &str, new: &str) -> bool {
        self.ask(HostKeyEvent {
            request_id: uuid::Uuid::new_v4().to_string(),
            host: host.into(),
            port,
            fingerprint: new.into(),
            old_fingerprint: Some(old.into()),
            kind: "changed",
        })
        .await
    }
}

#[tauri::command(rename_all = "snake_case")]
pub async fn approval_decide(
    state: tauri::State<'_, AppState>,
    approval_id: ApprovalId,
    decision: ApprovalDecision,
) -> Result<()> {
    let pending = state
        .terminal
        .approvals
        .lock()
        .unwrap()
        .remove(&approval_id);
    match pending {
        Some((_, tx)) => {
            let _ = tx.send(decision);
            Ok(())
        }
        None => Err(Error::NotFound("approval already resolved".into())),
    }
}

#[tauri::command(rename_all = "snake_case")]
pub async fn host_key_decision(
    state: tauri::State<'_, AppState>,
    request_id: String,
    accept: bool,
) -> Result<()> {
    if let Some(tx) = state.terminal.host_keys.lock().await.remove(&request_id) {
        let _ = tx.send(accept);
    }
    Ok(())
}

// ---------- opening sessions ----------

/// The UI stores `{ default_mode, deny_list: string[] }` under the `approvals` setting.
async fn approval_settings(state: &AppState) -> (ApprovalMode, Vec<regex::Regex>) {
    let value = {
        let vault = state.vault.lock().await;
        vault
            .get::<aht_vault::Setting>("approvals")
            .ok()
            .flatten()
            .map(|s| s.value)
    };
    let mode = value
        .as_ref()
        .and_then(|v| serde_json::from_value(v.get("default_mode")?.clone()).ok())
        .unwrap_or_default();
    let mut deny = aht_term::policy::default_deny_list();
    if let Some(list) = value
        .as_ref()
        .and_then(|v| v.get("deny_list")?.as_array().cloned())
    {
        deny.extend(
            list.iter()
                .filter_map(|p| p.as_str())
                .filter_map(|p| regex::Regex::new(p).ok()),
        );
    }
    (mode, deny)
}

/// `ask` answers login prompts for connections without saved credentials; without it (an agent
/// opening the connection) such a connection cannot log in.
async fn auth_material(
    state: &AppState,
    c: &Connection,
    ask: Option<&Arc<dyn KeyboardInteractiveHandler>>,
) -> Result<AuthMaterial> {
    Ok(match &c.auth {
        SshAuth::Password { secret } => {
            AuthMaterial::Password(Zeroizing::new(secret_value(state, *secret).await?))
        }
        SshAuth::Key { key, passphrase } => AuthMaterial::Key {
            pem: Zeroizing::new(secret_value(state, *key).await?),
            passphrase: match passphrase {
                Some(p) => Some(Zeroizing::new(secret_value(state, *p).await?)),
                None => None,
            },
        },
        SshAuth::Agent => AuthMaterial::Agent,
        SshAuth::Prompt => match ask {
            Some(handler) => AuthMaterial::KeyboardInteractive(handler.clone()),
            None => {
                return Err(Error::Auth(format!(
                    "{} has no saved password or key, so only you can log in to it: open it \
                     in Handrail and type the password",
                    c.name
                )))
            }
        },
    })
}

async fn load_connection(state: &AppState, id: ConnectionId) -> Result<Connection> {
    state
        .vault
        .lock()
        .await
        .get::<Connection>(&id.to_string())
        .map_err(Error::from)?
        .ok_or_else(|| Error::NotFound(format!("connection {id}")))
}

pub async fn open_ssh(
    app: &AppHandle,
    connection_id: ConnectionId,
    cols: u16,
    rows: u16,
    prompt_ref: Option<String>,
) -> Result<SessionInfo> {
    let state = app.state::<AppState>();
    let conn = load_connection(&state, connection_id).await?;
    if conn.kind != ConnectionKind::Ssh {
        return Err(Error::Invalid(format!(
            "{:?} connections are not supported yet",
            conn.kind
        )));
    }
    let ask: Option<Arc<dyn KeyboardInteractiveHandler>> = prompt_ref.map(|prompt_ref| {
        Arc::new(UiAuthPrompt {
            app: app.clone(),
            prompt_ref,
        }) as Arc<dyn KeyboardInteractiveHandler>
    });
    let mut params = SshParams::new(
        conn.host.clone(),
        conn.port,
        conn.user.clone(),
        auth_material(&state, &conn, ask.as_ref()).await?,
    );
    params.cols = cols;
    params.rows = rows;
    for jid in &conn.jump {
        let j = load_connection(&state, *jid).await?;
        params.jump.push(JumpHop {
            host: j.host.clone(),
            port: j.port,
            user: j.user.clone(),
            auth: auth_material(&state, &j, ask.as_ref()).await?,
        });
    }
    let store = KnownHosts::with_user_known_hosts(state.paths.data_dir.join("known_hosts"));
    let verifier = Arc::new(PromptingVerifier::new(
        store,
        Arc::new(UiHostKeyPrompt { app: app.clone() }),
    ));
    let (shell, events) = aht_ssh::connect_shell(&params, verifier).await?;
    let (default, deny_list) = approval_settings(&state).await;
    let mode = conn.approval_mode.unwrap_or(default);
    let cfg = TermConfig {
        connection_id: Some(conn.id),
        title: conn.name.clone(),
        policy: Policy {
            mode,
            env: conn.env,
            deny_list,
        },
        ..TermConfig::default()
    };
    let ssh = shell.connection().clone();
    let info = register(app, Arc::new(shell), events, cfg, vec![], vec![]).await;
    if let Some(e) = hub(app).sessions.write().await.get_mut(&info.session_id) {
        e.ssh = Some(ssh.clone());
        e.tunnels = crate::tunnels::starting(&conn.tunnels);
    }
    // In the background: a port still held by the previous session (reconnect) is waited for,
    // and must not hold up the terminal.
    if !conn.tunnels.is_empty() {
        let (app, id, tunnels) = (app.clone(), info.session_id, conn.tunnels.clone());
        tokio::spawn(async move {
            let (forwards, status) = crate::tunnels::start(&ssh, &tunnels).await;
            if let Some(e) = hub(&app).sessions.write().await.get_mut(&id) {
                e.forwards = forwards;
                e.tunnels = status.clone();
                crate::tunnels::emit(&app, id, status);
            }
        });
    }
    Ok(info)
}

async fn open_local(
    app: &AppHandle,
    spec: aht_ssh::LocalShellSpec,
    title: String,
    inject: bool,
    tokens: Vec<McpToken>,
    temp_files: Vec<PathBuf>,
) -> Result<SessionInfo> {
    let (pty, events) = tokio::task::spawn_blocking(move || aht_ssh::spawn_local(spec))
        .await
        .map_err(err)??;
    let (mode, deny_list) = approval_settings(&app.state::<AppState>()).await;
    let cfg = TermConfig {
        title,
        inject,
        // Panes hosting an agent CLI (native harness, login) are never agent targets.
        agent_target: inject,
        // ConPTY repaints from its own buffer, which would replay the hidden hook lines.
        clear_after_inject: cfg!(windows),
        policy: Policy {
            mode,
            deny_list,
            ..Policy::default()
        },
        ..TermConfig::default()
    };
    Ok(register(app, Arc::new(pty), events, cfg, tokens, temp_files).await)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn open_session(
    app: AppHandle,
    connection_id: ConnectionId,
    cols: u16,
    rows: u16,
    prompt_ref: Option<String>,
) -> Result<SessionInfo> {
    open_ssh(&app, connection_id, cols, rows, prompt_ref).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn open_local_session(
    app: AppHandle,
    shell: Option<String>,
    cols: u16,
    rows: u16,
) -> Result<SessionInfo> {
    let shells = tokio::task::spawn_blocking(aht_ssh::default_local_shells)
        .await
        .map_err(err)?;
    let chosen = match &shell {
        Some(id) => shells.iter().find(|s| &s.id == id),
        None => shells.first(),
    }
    .ok_or_else(|| Error::NotFound("no local shell found".into()))?;
    let title = chosen.label.clone();
    open_local(
        &app,
        chosen.to_spec(cols, rows),
        title,
        true,
        vec![],
        vec![],
    )
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn session_write(
    state: tauri::State<'_, AppState>,
    session_id: SessionId,
    data_b64: String,
) -> Result<()> {
    let bytes = B64
        .decode(data_b64)
        .map_err(|e| Error::Invalid(e.to_string()))?;
    state
        .terminal
        .session(session_id)
        .await?
        .write_user(&bytes)
        .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn session_resize(
    state: tauri::State<'_, AppState>,
    session_id: SessionId,
    cols: u16,
    rows: u16,
) -> Result<()> {
    state
        .terminal
        .session(session_id)
        .await?
        .resize(cols, rows)
        .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn close_session(state: tauri::State<'_, AppState>, session_id: SessionId) -> Result<()> {
    let hub = &state.terminal;
    let entry = hub.sessions.write().await.remove(&session_id);
    let sftps: Vec<_> = {
        let mut map = hub.sftp.lock().await;
        [false, true]
            .iter()
            .filter_map(|root| map.remove(&(session_id, *root)))
            .collect()
    };
    for sftp in sftps {
        sftp.close().await;
    }
    hub.sftp_root_active.lock().unwrap().remove(&session_id);
    let pending: Vec<_> = {
        let mut map = hub.approvals.lock().unwrap();
        let ids: Vec<_> = map
            .iter()
            .filter(|(_, (s, _))| *s == session_id)
            .map(|(id, _)| *id)
            .collect();
        ids.into_iter().filter_map(|id| map.remove(&id)).collect()
    };
    drop(pending);
    let runs: Vec<Run> = {
        let mut map = hub.runs.lock().await;
        let ids: Vec<_> = map
            .iter()
            .filter(|(_, r)| r.session_id == session_id)
            .map(|(id, _)| id.clone())
            .collect();
        ids.into_iter().filter_map(|id| map.remove(&id)).collect()
    };
    for r in runs {
        match r.kind {
            RunKind::BuiltIn(c) => c.cancel(),
            RunKind::Harness(h) => h.cancel().await,
        }
    }
    let prefix = format!("{session_id}:");
    hub.agents
        .lock()
        .await
        .retain(|k, _| !k.starts_with(&prefix));
    let chats: Vec<HarnessChat> = {
        let mut map = hub.harness_chats.lock().await;
        let keys: Vec<_> = map
            .keys()
            .filter(|(s, _)| *s == session_id)
            .cloned()
            .collect();
        keys.into_iter().filter_map(|k| map.remove(&k)).collect()
    };
    for chat in chats {
        chat.run.cancel().await;
        hub.revoke(&chat.token);
    }
    if let Some(e) = entry {
        for t in &e.tokens {
            hub.revoke(t);
        }
        aht_harness_cleanup(&e.temp_files);
        let _ = e.term.close().await;
    }
    Ok(())
}

fn aht_harness_cleanup(files: &[PathBuf]) {
    for f in files {
        let _ = std::fs::remove_file(f);
    }
}

#[tauri::command(rename_all = "snake_case")]
pub async fn list_sessions(state: tauri::State<'_, AppState>) -> Result<Vec<SessionInfo>> {
    Ok(state
        .terminal
        .sessions
        .read()
        .await
        .values()
        .map(|e| e.term.info())
        .collect())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn set_approval_mode(
    state: tauri::State<'_, AppState>,
    session_id: SessionId,
    mode: ApprovalMode,
) -> Result<()> {
    let s = state.terminal.session(session_id).await?;
    let mut p = s.policy();
    p.mode = mode;
    s.set_policy(p);
    let _ = state.terminal.app().emit("session://state", s.info());
    Ok(())
}

// ---------- TerminalTools (shared by built-in agent and MCP) ----------

pub struct Tools {
    app: AppHandle,
}

impl Tools {
    /// The actor's scope plus any sessions it opened itself.
    fn scoped(&self, actor: &Actor) -> Actor {
        let mut a = actor.clone();
        if let Some(scope) = a.session_scope.as_mut() {
            if let Some(extra) = hub(&self.app).grants.lock().unwrap().get(&actor.id) {
                scope.extend(extra.iter().copied());
            }
        }
        a
    }
}

#[async_trait]
impl TerminalTools for Tools {
    async fn list_connections(&self, _actor: &Actor) -> Result<Vec<ConnectionSummary>> {
        let state = self.app.state::<AppState>();
        let conns = state
            .vault
            .lock()
            .await
            .list::<Connection>()
            .map_err(Error::from)?;
        Ok(conns
            .into_iter()
            .map(|c| ConnectionSummary {
                id: c.id,
                name: c.name,
                kind: c.kind,
                host: c.host,
                user: c.user,
                env: c.env,
                group_id: c.group_id,
                tags: c.tags,
            })
            .collect())
    }

    async fn open_session(&self, actor: &Actor, connection: ConnectionId) -> Result<SessionInfo> {
        let info = open_ssh(&self.app, connection, 120, 32, None).await?;
        hub(&self.app)
            .grants
            .lock()
            .unwrap()
            .entry(actor.id.clone())
            .or_default()
            .insert(info.session_id);
        // The UI adds a tab for sessions an agent opened.
        let _ = self.app.emit("session://opened", info.clone());
        Ok(info)
    }

    async fn list_sessions(&self, actor: &Actor) -> Result<Vec<SessionInfo>> {
        let actor = self.scoped(actor);
        Ok(hub(&self.app)
            .sessions
            .read()
            .await
            .values()
            .filter(|e| e.term.agent_target())
            .map(|e| e.term.info())
            .filter(|i| actor.may_access(i.session_id))
            .collect())
    }

    async fn session_info(&self, actor: &Actor, session: SessionId) -> Result<SessionInfo> {
        if !self.scoped(actor).may_access(session) {
            return Err(Error::Denied("session outside the actor's scope".into()));
        }
        Ok(hub(&self.app).session(session).await?.info())
    }

    async fn run_command(&self, actor: &Actor, req: RunCommandRequest) -> Result<CommandResult> {
        let s = hub(&self.app).session(req.session_id).await?;
        s.run_command(
            &self.scoped(actor),
            req,
            &UiApprover {
                app: self.app.clone(),
            },
        )
        .await
    }

    async fn read_output(
        &self,
        actor: &Actor,
        output: OutputId,
        offset: usize,
        limit: usize,
    ) -> Result<OutputChunk> {
        let actor = self.scoped(actor);
        for e in hub(&self.app).sessions.read().await.values() {
            if !actor.may_access(e.term.id()) {
                continue;
            }
            if let Ok(chunk) = e.term.read_output(output, offset, limit) {
                return Ok(chunk);
            }
        }
        Err(Error::NotFound(format!("output {output}")))
    }

    async fn send_keys(
        &self,
        actor: &Actor,
        session: SessionId,
        keys: Vec<ControlKey>,
    ) -> Result<()> {
        hub(&self.app)
            .session(session)
            .await?
            .send_keys(&self.scoped(actor), &keys)
            .await
    }

    async fn propose_file_edit(
        &self,
        actor: &Actor,
        req: FileEditRequest,
    ) -> Result<FileEditResult> {
        let actor = self.scoped(actor);
        if !actor.may_access(req.session_id) {
            return Err(Error::Denied("session outside the actor's scope".into()));
        }
        propose_edit(&self.app, &actor, req).await
    }
}

// ---------- file edits proposed by agents ----------

/// Largest file an agent may propose.
const MAX_PROPOSED_FILE: usize = 1024 * 1024;
const EDIT_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Shown in the editor as a diff against the current file.
#[derive(Clone, Serialize)]
struct EditProposal {
    proposal_id: String,
    session_id: SessionId,
    path: String,
    /// Proposed through the root (sudo) SFTP server: saving needs it too.
    root: bool,
    /// Current contents and stamp; `None` for a new file.
    original: Option<String>,
    stamp: Option<crate::sftp::Stamp>,
    proposed: String,
    reason: Option<String>,
    actor_label: String,
}

/// Removes a pending edit when its wait ends for any reason; the UI then drops its diff.
struct EditGuard<'a> {
    app: &'a AppHandle,
    id: String,
}

impl Drop for EditGuard<'_> {
    fn drop(&mut self) {
        if hub(self.app)
            .edits
            .lock()
            .unwrap()
            .remove(&self.id)
            .is_some()
        {
            let _ = self.app.emit("edit://resolved", self.id.clone());
        }
    }
}

async fn propose_edit(
    app: &AppHandle,
    actor: &Actor,
    req: FileEditRequest,
) -> Result<FileEditResult> {
    if !req.path.starts_with('/') {
        return Err(Error::Invalid("path must be absolute".into()));
    }
    if req.content.len() > MAX_PROPOSED_FILE {
        return Err(Error::Invalid("files over 1 MiB cannot be proposed".into()));
    }
    let state = app.state::<AppState>();
    // Read (and later save) as root while the user's file browser is in root mode.
    let root = hub(app)
        .sftp_root_active
        .lock()
        .unwrap()
        .contains(&req.session_id);
    let sftp = crate::sftp::client(&state, req.session_id, Some(root)).await?;
    let (original, stamp) = match crate::sftp::read_text(&sftp, &req.path).await {
        Ok(t) => (Some(t.text), Some(t.stamp)),
        Err(Error::NotFound(_)) => (None, None),
        Err(e) => return Err(e),
    };
    let id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = oneshot::channel();
    hub(app)
        .edits
        .lock()
        .unwrap()
        .insert(id.clone(), (req.path.clone(), tx));
    let _guard = EditGuard {
        app,
        id: id.clone(),
    };
    let _ = app.emit(
        "edit://proposal",
        EditProposal {
            proposal_id: id,
            session_id: req.session_id,
            path: req.path.clone(),
            root,
            original,
            stamp,
            proposed: req.content,
            reason: req.reason,
            actor_label: actor.label.clone(),
        },
    );
    let rejected = |note: &str| FileEditResult {
        outcome: FileEditOutcome::Rejected,
        path: req.path.clone(),
        note: Some(note.into()),
    };
    Ok(match tokio::time::timeout(EDIT_TIMEOUT, rx).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => rejected("cancelled"),
        Err(_) => rejected("no answer within 30 minutes"),
    })
}

/// The user's answer to a proposed edit (after saving, for an accepted one).
#[tauri::command(rename_all = "snake_case")]
pub async fn edit_proposal_decide(
    state: tauri::State<'_, AppState>,
    proposal_id: String,
    outcome: FileEditOutcome,
    note: Option<String>,
) -> Result<()> {
    let pending = state.terminal.edits.lock().unwrap().remove(&proposal_id);
    if let Some((path, tx)) = pending {
        let _ = tx.send(FileEditResult {
            outcome,
            path,
            note: note.filter(|n| !n.trim().is_empty()),
        });
    }
    Ok(())
}

// ---------- agents ----------

#[derive(Serialize)]
pub struct RunStarted {
    run_id: String,
}

fn emit_run(app: &AppHandle, run_id: &str, session_id: SessionId, event: AgentEvent) {
    let _ = app.emit(
        "agent://event",
        AgentRunEvent {
            run_id: run_id.to_string(),
            session_id,
            event,
        },
    );
}

#[tauri::command(rename_all = "snake_case")]
pub async fn agent_send(
    app: AppHandle,
    session_id: SessionId,
    brain: Brain,
    message: String,
) -> Result<RunStarted> {
    let run_id = uuid::Uuid::new_v4().to_string();
    match brain {
        Brain::BuiltIn { provider, model } => {
            start_builtin(
                &app,
                &run_id,
                session_id,
                provider.to_string(),
                model,
                message,
            )
            .await?
        }
        Brain::Harness { profile } => {
            start_harness(&app, &run_id, session_id, profile.to_string(), message).await?
        }
    }
    Ok(RunStarted { run_id })
}

async fn start_builtin(
    app: &AppHandle,
    run_id: &str,
    session_id: SessionId,
    provider_id: String,
    model: String,
    message: String,
) -> Result<()> {
    let state = app.state::<AppState>();
    let hub = &state.terminal;
    let info = hub.session(session_id).await?.info();
    let key = format!("{session_id}:{provider_id}:{model}");
    let agent = {
        let mut agents = hub.agents.lock().await;
        match agents.get(&key) {
            Some(a) => a.clone(),
            None => {
                let (provider, client) =
                    crate::cmd_misc::provider_client(&state, &provider_id).await?;
                let actor = Actor {
                    id: format!("builtin:{session_id}:{model}"),
                    label: format!("{} · {model}", provider.name),
                    session_scope: Some(vec![session_id]),
                };
                let a = Arc::new(Mutex::new(aht_agent::Agent::new(
                    client,
                    Arc::new(Tools { app: app.clone() }),
                    actor,
                    aht_agent::AgentConfig::new(model.clone()),
                )));
                agents.insert(key, a.clone());
                a
            }
        }
    };
    let cancel = CancellationToken::new();
    hub.runs.lock().await.insert(
        run_id.to_string(),
        Run {
            session_id,
            kind: RunKind::BuiltIn(cancel.clone()),
        },
    );
    let (tx, mut rx) = mpsc::channel(256);
    let a = app.clone();
    let rid = run_id.to_string();
    tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            emit_run(&a, &rid, session_id, ev);
        }
        self::hub(&a).runs.lock().await.remove(&rid);
    });
    let a = app.clone();
    let rid = run_id.to_string();
    tokio::spawn(async move {
        let Ok(mut agent) = agent.try_lock_owned() else {
            emit_run(
                &a,
                &rid,
                session_id,
                AgentEvent::Error {
                    message: "the agent is still answering".into(),
                    retryable: true,
                },
            );
            emit_run(&a, &rid, session_id, AgentEvent::Done);
            return;
        };
        // Errors are already reported through the event stream.
        let _ = agent.run(&message, Some(info), tx, cancel).await;
    });
    Ok(())
}

async fn load_profile(state: &AppState, id: &str) -> Result<HarnessProfile> {
    state
        .vault
        .lock()
        .await
        .get::<HarnessProfile>(id)
        .map_err(Error::from)?
        .ok_or_else(|| Error::NotFound(format!("harness profile {id}")))
}

fn session_summary(info: &SessionInfo) -> String {
    let mut s = format!(
        "Live terminal session id {} titled \"{}\"",
        info.session_id, info.title
    );
    if let (Some(u), Some(h)) = (&info.user, &info.host) {
        s.push_str(&format!(", logged in as {u}@{h}"));
    }
    if let Some(os) = &info.os {
        s.push_str(&format!(", OS {os}"));
    }
    if let Some(sh) = &info.shell {
        s.push_str(&format!(", shell {sh}"));
    }
    s.push_str(&format!(
        ", environment {:?}. Use session_id {} with the handrail tools (run_command, read_output, \
send_keys). Commands are typed into the user's live terminal and may need their approval.",
        info.env, info.session_id
    ));
    s
}

async fn launch_context(
    app: &AppHandle,
    profile: &HarnessProfile,
    info: &SessionInfo,
) -> Result<(LaunchContext, McpToken)> {
    let state = app.state::<AppState>();
    let mcp = state.terminal.mcp().await?;
    let run_id = uuid::Uuid::new_v4().to_string();
    let token = mcp.issue_token(Actor {
        id: format!("harness:{run_id}"),
        label: format!("{} ({})", harness_label(profile.harness), profile.name),
        session_scope: Some(vec![info.session_id]),
    });
    let base = state.paths.profiles_dir.clone();
    let p = profile.clone();
    let prepared = tokio::task::spawn_blocking(move || aht_harness::prepare_profile_dir(&base, &p))
        .await
        .map_err(err)
        .and_then(|r| r);
    let profile_dir = match prepared {
        Ok(dir) => dir,
        Err(e) => {
            mcp.revoke(&token);
            return Err(e);
        }
    };
    let ctx = LaunchContext {
        run_id,
        mcp_url: mcp.url(),
        token_env_var: aht_harness::DEFAULT_TOKEN_ENV_VAR.to_string(),
        token: token.as_str().to_string(),
        session_summary: session_summary(info),
        profile_dir,
        plugin_dirs: aht_harness::resolve_plugin_dirs(profile.harness, &profile.plugins),
    };
    Ok((ctx, token))
}

fn harness_label(kind: HarnessKind) -> &'static str {
    match kind {
        HarnessKind::ClaudeCode => "Claude Code",
        HarnessKind::Codex => "Codex",
        HarnessKind::OpenCode => "OpenCode",
    }
}

async fn start_harness(
    app: &AppHandle,
    run_id: &str,
    session_id: SessionId,
    profile_id: String,
    message: String,
) -> Result<()> {
    let state = app.state::<AppState>();
    let hub = &state.terminal;
    let profile = load_profile(&state, &profile_id).await?;
    if profile.display == HarnessDisplay::Native {
        return Err(Error::Invalid(
            "this profile runs in Native mode — open it in a pane next to the terminal".into(),
        ));
    }
    // Claude Code keeps one streaming process per tab, so follow-ups keep their context.
    let key = (session_id, profile_id.clone());
    if profile.harness == HarnessKind::ClaudeCode {
        let live = hub
            .harness_chats
            .lock()
            .await
            .get(&key)
            .filter(|c| !c.run.is_finished())
            .map(|c| (c.run.clone(), c.run_id.clone()));
        if let Some((run, current)) = live {
            run.send_user_message(&message).await?;
            *current.lock().unwrap() = run_id.to_string();
            hub.runs.lock().await.insert(
                run_id.to_string(),
                Run {
                    session_id,
                    kind: RunKind::Harness(run),
                },
            );
            return Ok(());
        }
    }
    let info = hub.session(session_id).await?.info();
    let (ctx, token) = launch_context(app, &profile, &info).await?;
    let spec = aht_harness::build_launch(profile.harness, &profile, LaunchMode::Headless, &ctx);
    let (run, mut rx) = match HeadlessRun::start(spec, &message).await {
        Ok(r) => r,
        Err(e) => {
            hub.revoke(&token);
            return Err(e);
        }
    };
    let run = Arc::new(run);
    hub.runs.lock().await.insert(
        run_id.to_string(),
        Run {
            session_id,
            kind: RunKind::Harness(run.clone()),
        },
    );
    let current = Arc::new(std::sync::Mutex::new(run_id.to_string()));
    let old = hub.harness_chats.lock().await.insert(
        key,
        HarnessChat {
            run: run.clone(),
            token: token.clone(),
            run_id: current.clone(),
        },
    );
    if let Some(old) = old {
        old.run.cancel().await;
        hub.revoke(&old.token);
    }
    // Events are tagged with the run id the UI is currently tracking for this chat.
    let a = app.clone();
    tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            let rid = current.lock().unwrap().clone();
            emit_run(&a, &rid, session_id, ev);
        }
        // The process exited: its MCP token dies with it.
        let hub = self::hub(&a);
        hub.revoke(&token);
        hub.runs.lock().await.retain(|_, r| match &r.kind {
            RunKind::Harness(h) => !Arc::ptr_eq(h, &run),
            RunKind::BuiltIn(_) => true,
        });
    });
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn agent_cancel(state: tauri::State<'_, AppState>, run_id: String) -> Result<()> {
    let run = state.terminal.runs.lock().await.remove(&run_id);
    match run.map(|r| r.kind) {
        Some(RunKind::BuiltIn(c)) => c.cancel(),
        Some(RunKind::Harness(r)) => r.cancel().await,
        None => {}
    }
    Ok(())
}

/// Forget a tab's built-in agent conversation.
#[tauri::command(rename_all = "snake_case")]
pub async fn agent_reset(state: tauri::State<'_, AppState>, session_id: SessionId) -> Result<()> {
    let prefix = format!("{session_id}:");
    state
        .terminal
        .agents
        .lock()
        .await
        .retain(|k, _| !k.starts_with(&prefix));
    Ok(())
}

// ---------- native harness panes & login ----------

fn local_spec_from(spec: &LaunchSpec, cols: u16, rows: u16) -> Result<aht_ssh::LocalShellSpec> {
    let program = aht_harness::resolve_program(&spec.program);
    let (program, mut args) = wrap_shim(program);
    args.extend(spec.args.iter().cloned());
    for (path, content) in &spec.files {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, content)?;
    }
    let mut local = aht_ssh::LocalShellSpec::new(program, args);
    local.cwd = spec.cwd.clone();
    local.env = spec.env.clone();
    local.env_remove = spec.env_remove.clone();
    local.cols = cols;
    local.rows = rows;
    Ok(local)
}

/// npm installs `.cmd` / `.ps1` shims on Windows, which CreateProcess cannot run directly.
fn wrap_shim(program: PathBuf) -> (String, Vec<String>) {
    let ext = program
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let p = program.to_string_lossy().to_string();
    match ext.as_deref() {
        Some("cmd") | Some("bat") => ("cmd.exe".into(), vec!["/d".into(), "/c".into(), p]),
        Some("ps1") => (
            "powershell.exe".into(),
            vec![
                "-NoProfile".into(),
                "-ExecutionPolicy".into(),
                "Bypass".into(),
                "-File".into(),
                p,
            ],
        ),
        _ => (p, vec![]),
    }
}

#[tauri::command(rename_all = "snake_case")]
pub async fn harness_open_native(
    app: AppHandle,
    session_id: SessionId,
    profile_id: String,
    cols: u16,
    rows: u16,
) -> Result<SessionInfo> {
    let state = app.state::<AppState>();
    let profile = load_profile(&state, &profile_id).await?;
    let info = state.terminal.session(session_id).await?.info();
    let (ctx, token) = launch_context(&app, &profile, &info).await?;
    let spec = aht_harness::native_spec(profile.harness, &profile, &ctx);
    let files: Vec<PathBuf> = spec.files.iter().map(|(p, _)| p.clone()).collect();
    let title = format!("{} · {}", harness_label(profile.harness), info.title);
    // A TUI pane: no shell integration, and agent tools never target it.
    let opened = match local_spec_from(&spec, cols, rows) {
        Ok(local) => {
            open_local(
                &app,
                local,
                title,
                false,
                vec![token.clone()],
                files.clone(),
            )
            .await
        }
        Err(e) => Err(e),
    };
    if opened.is_err() {
        state.terminal.revoke(&token);
        aht_harness_cleanup(&files);
    }
    opened
}

#[tauri::command(rename_all = "snake_case")]
pub async fn harness_login(
    app: AppHandle,
    kind: HarnessKind,
    cols: u16,
    rows: u16,
) -> Result<SessionInfo> {
    let spec = aht_harness::login_spec(kind);
    let local = local_spec_from(&spec, cols, rows)?;
    open_local(
        &app,
        local,
        format!("{} sign-in", harness_label(kind)),
        false,
        vec![],
        vec![],
    )
    .await
}
