//! SSH client: connect (with ProxyJump), authenticate, open PTY shells and exec channels.

use crate::auth::{AuthMaterial, KeyboardInteractiveHandler, KiPrompt};
use crate::forward::{connect_forwarded, RemoteForwards};
use crate::verify::{HostKeyDecision, HostKeyVerifier};
use aht_core::terminal::{Transport, TransportEvent};
use aht_core::{Error, Result};
use async_trait::async_trait;
use russh::client::AuthResult;
use russh::client::{
    self, ChannelOpenHandle, DisconnectReason, Handle, KeyboardInteractiveAuthResponse, Msg,
    Session,
};
use russh::keys::agent::client::{AgentClient, AgentStream};
use russh::keys::agent::AgentIdentity;
use russh::keys::{Algorithm, HashAlg, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate};
use russh::{AgentAuthError, Channel, ChannelMsg, ChannelReadHalf, ChannelWriteHalf, MethodKind};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

/// One SSH endpoint (a jump hop or the final target).
#[derive(Clone, Debug)]
pub struct JumpHop {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: AuthMaterial,
}

/// Everything needed to open an interactive SSH session.
#[derive(Clone, Debug)]
pub struct SshParams {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: AuthMaterial,
    /// ProxyJump chain, first hop first. Each hop is verified and authenticated on its own.
    pub jump: Vec<JumpHop>,
    /// Keepalive interval; after 3 unanswered keepalives the connection is considered dead.
    pub keepalive: Option<Duration>,
    /// Timeout per hop for TCP connect + handshake + auth, not counting time spent waiting on
    /// the user (host key confirmation, password or code prompts).
    pub connect_timeout: Duration,
    pub term: String,
    pub cols: u16,
    pub rows: u16,
    /// Environment variables requested for the shell (servers may refuse them silently).
    pub env: Vec<(String, String)>,
}

impl SshParams {
    pub fn new(
        host: impl Into<String>,
        port: u16,
        user: impl Into<String>,
        auth: AuthMaterial,
    ) -> Self {
        Self {
            host: host.into(),
            port,
            user: user.into(),
            auth,
            jump: Vec::new(),
            keepalive: Some(Duration::from_secs(30)),
            connect_timeout: Duration::from_secs(15),
            term: "xterm-256color".into(),
            cols: 80,
            rows: 24,
            env: Vec::new(),
        }
    }

    fn target(&self) -> JumpHop {
        JumpHop {
            host: self.host.clone(),
            port: self.port,
            user: self.user.clone(),
            auth: self.auth.clone(),
        }
    }
}

type SharedSlot<T> = Arc<Mutex<Option<T>>>;

fn put<T>(slot: &SharedSlot<T>, v: T) {
    *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(v);
}

fn get<T: Clone>(slot: &SharedSlot<T>) -> Option<T> {
    slot.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Time a connection attempt spent waiting on the user, so the connect timeout covers only the
/// network: someone comparing a fingerprint or typing a password and an OTP is not a dead host.
#[derive(Clone, Default)]
pub(crate) struct UserWait(Arc<Mutex<(Duration, Option<tokio::time::Instant>)>>);

impl UserWait {
    /// Run `fut` (a prompt) with the connect deadline paused.
    pub(crate) async fn around<F: std::future::Future>(&self, fut: F) -> F::Output {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).1 = Some(tokio::time::Instant::now());
        let out = fut.await;
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(since) = g.1.take() {
            g.0 += since.elapsed();
        }
        out
    }

    /// Total time waited so far, including a prompt still open.
    fn waited(&self) -> Duration {
        let g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        g.0 + g.1.map_or(Duration::ZERO, |s| s.elapsed())
    }
}

/// Like `tokio::time::timeout`, but the clock stops while `user` is waiting on a prompt.
pub(crate) async fn timeout_excluding_user<F: std::future::Future>(
    limit: Duration,
    user: &UserWait,
    fut: F,
) -> Option<F::Output> {
    let start = tokio::time::Instant::now();
    tokio::pin!(fut);
    loop {
        // While a prompt is open the deadline keeps moving, so this never fires during one.
        let deadline = start + limit + user.waited();
        if deadline <= tokio::time::Instant::now() {
            return None;
        }
        tokio::select! {
            r = &mut fut => return Some(r),
            _ = tokio::time::sleep_until(deadline) => {}
        }
    }
}

pub(crate) struct ClientHandler {
    host: String,
    port: u16,
    verifier: Arc<dyn HostKeyVerifier>,
    decision: SharedSlot<HostKeyDecision>,
    lost: SharedSlot<String>,
    remote: RemoteForwards,
    user: UserWait,
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> std::result::Result<bool, Self::Error> {
        let key = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            PublicKeyOrCertificate::Certificate(c) => PublicKey::from(c.public_key().clone()),
        };
        let d = self
            .user
            .around(self.verifier.verify(&self.host, self.port, &key))
            .await;
        let ok = matches!(
            d,
            HostKeyDecision::Accept | HostKeyDecision::ReplaceAndTrust
        );
        put(&self.decision, d);
        Ok(ok)
    }

    async fn disconnected(
        &mut self,
        reason: DisconnectReason<Self::Error>,
    ) -> std::result::Result<(), Self::Error> {
        match reason {
            DisconnectReason::ReceivedDisconnect(info) => {
                put(&self.lost, format!("server disconnected: {}", info.message));
                Ok(())
            }
            DisconnectReason::Error(e) => {
                put(&self.lost, format!("connection lost: {e}"));
                Err(e)
            }
        }
    }

    /// A connection to a port the server listens on for us (`ssh -R`).
    #[allow(clippy::too_many_arguments)]
    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<Msg>,
        connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> std::result::Result<(), Self::Error> {
        // Not a port we asked for: dropping `reply` refuses the channel.
        if let Some(dest) = self.remote.get(connected_address, connected_port) {
            reply.accept().await;
            connect_forwarded(channel, dest);
        }
        Ok(())
    }
}

/// `default` with the key types already recorded for the host first, like OpenSSH: a server
/// with several host keys then shows the one we know, so a different type means a real change.
fn host_key_order(default: &[Algorithm], known: &[Algorithm]) -> Vec<Algorithm> {
    // An RSA key signs with any RSA hash.
    let family = |a: &Algorithm| match a {
        Algorithm::Rsa { .. } => Algorithm::Rsa { hash: None },
        other => other.clone(),
    };
    let known: Vec<Algorithm> = known.iter().map(family).collect();
    let (mut first, rest): (Vec<Algorithm>, Vec<Algorithm>) = default
        .iter()
        .cloned()
        .partition(|a| known.contains(&family(a)));
    first.extend(rest);
    first
}

pub(crate) fn net(e: impl std::fmt::Display) -> Error {
    Error::Network(e.to_string())
}

/// An authenticated SSH connection (possibly tunnelled through jump hosts).
pub struct SshConnection {
    pub(crate) handle: Handle<ClientHandler>,
    /// Jump-hop connections; must outlive `handle`.
    _hops: Vec<Handle<ClientHandler>>,
    lost: SharedSlot<String>,
    remote: RemoteForwards,
    label: String,
}

impl std::fmt::Debug for SshConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshConnection")
            .field("target", &self.label)
            .finish()
    }
}

impl SshConnection {
    /// Connect, verify host keys and authenticate every hop, ending at the target.
    pub async fn connect(params: &SshParams, verifier: Arc<dyn HostKeyVerifier>) -> Result<Self> {
        let config_for = |ep: &JumpHop| {
            let mut c = client::Config {
                keepalive_interval: params.keepalive,
                keepalive_max: 3,
                nodelay: true,
                ..Default::default()
            };
            let known = verifier.known_algorithms(&ep.host, ep.port);
            c.preferred.key = host_key_order(&c.preferred.key, &known).into();
            Arc::new(c)
        };
        let mut endpoints = params.jump.clone();
        endpoints.push(params.target());

        let mut handles: Vec<Handle<ClientHandler>> = Vec::new();
        let mut last_lost = None;
        let remote = RemoteForwards::default();
        for ep in &endpoints {
            let config = config_for(ep);
            let decision: SharedSlot<HostKeyDecision> = Default::default();
            let lost: SharedSlot<String> = Default::default();
            let user = UserWait::default();
            let handler = ClientHandler {
                host: ep.host.clone(),
                port: ep.port,
                verifier: verifier.clone(),
                decision: decision.clone(),
                lost: lost.clone(),
                remote: remote.clone(),
                user: user.clone(),
            };
            let prev = handles.last();
            let attempt = async {
                let res = match prev {
                    None => {
                        client::connect(config.clone(), (ep.host.as_str(), ep.port), handler).await
                    }
                    Some(prev) => {
                        let ch = prev
                            .channel_open_direct_tcpip(
                                ep.host.clone(),
                                ep.port as u32,
                                "127.0.0.1",
                                0,
                            )
                            .await
                            .map_err(|e| {
                                net(format!("jump to {}:{} failed: {e}", ep.host, ep.port))
                            })?;
                        client::connect_stream(config.clone(), ch.into_stream(), handler).await
                    }
                };
                let mut h = res.map_err(|e| match get(&decision) {
                    Some(HostKeyDecision::Mismatch { old, new }) => Error::HostKeyMismatch(
                        format!("{}:{} (recorded {old}, presented {new})", ep.host, ep.port),
                    ),
                    Some(HostKeyDecision::Reject) => {
                        Error::Denied(format!("host key for {}:{} not trusted", ep.host, ep.port))
                    }
                    _ => net(format!("{}:{}: {e}", ep.host, ep.port)),
                })?;
                authenticate(&mut h, ep, &user).await?;
                Ok::<_, Error>(h)
            };
            let h = timeout_excluding_user(params.connect_timeout, &user, attempt)
                .await
                .ok_or_else(|| net(format!("{}:{}: connect timed out", ep.host, ep.port)))??;
            handles.push(h);
            last_lost = Some(lost);
        }
        let handle = handles.pop().expect("at least one endpoint");
        Ok(Self {
            handle,
            _hops: handles,
            lost: last_lost.expect("at least one endpoint"),
            remote,
            label: format!("{}@{}:{}", params.user, params.host, params.port),
        })
    }

    pub(crate) fn remote_forwards(&self) -> &RemoteForwards {
        &self.remote
    }

    pub fn is_closed(&self) -> bool {
        self.handle.is_closed()
    }

    /// Why the connection ended, if it has.
    pub fn disconnect_reason(&self) -> Option<String> {
        get(&self.lost)
    }

    /// Open a session channel with a PTY and interactive shell.
    pub async fn open_shell(
        self: &Arc<Self>,
        term: &str,
        cols: u16,
        rows: u16,
        env: &[(String, String)],
    ) -> Result<(SshShell, mpsc::Receiver<TransportEvent>)> {
        let ch = self.handle.channel_open_session().await.map_err(net)?;
        let (mut read, write) = ch.split();
        for (k, v) in env {
            let _ = write.set_env(false, k.clone(), v.clone()).await;
        }
        let mut pending = Vec::new();
        write
            .request_pty(true, term, cols as u32, rows as u32, 0, 0, &[])
            .await
            .map_err(net)?;
        wait_reply(&mut read, &mut pending, "pty request").await?;
        write.request_shell(true).await.map_err(net)?;
        wait_reply(&mut read, &mut pending, "shell request").await?;

        let (tx, rx) = mpsc::channel(256);
        for d in pending {
            let _ = tx.send(TransportEvent::Data(d)).await;
        }
        let conn = self.clone();
        let closed = Arc::new(tokio::sync::Notify::new());
        let closed2 = closed.clone();
        tokio::spawn(async move {
            let mut status = None;
            loop {
                let msg = tokio::select! {
                    m = read.wait() => m,
                    _ = closed2.notified() => {
                        let _ = tx.send(TransportEvent::Closed { exit_status: status }).await;
                        break;
                    }
                };
                match msg {
                    Some(ChannelMsg::Data { data })
                    | Some(ChannelMsg::ExtendedData { data, .. }) => {
                        if tx.send(TransportEvent::Data(data.to_vec())).await.is_err() {
                            break;
                        }
                    }
                    Some(ChannelMsg::ExitStatus { exit_status }) => status = Some(exit_status),
                    Some(ChannelMsg::Close) => {
                        let _ = tx
                            .send(TransportEvent::Closed {
                                exit_status: status,
                            })
                            .await;
                        break;
                    }
                    Some(_) => {}
                    None => {
                        if status.is_none() {
                            let why = conn
                                .disconnect_reason()
                                .unwrap_or_else(|| "connection lost".into());
                            let _ = tx.send(TransportEvent::Error(why)).await;
                        }
                        let _ = tx
                            .send(TransportEvent::Closed {
                                exit_status: status,
                            })
                            .await;
                        break;
                    }
                }
            }
        });
        Ok((
            SshShell {
                write,
                closed,
                connection: self.clone(),
            },
            rx,
        ))
    }

    /// Run `command` on a fresh session channel and collect its output.
    pub async fn exec(&self, command: &str) -> Result<ExecOutput> {
        self.exec_limited(command, None, usize::MAX).await
    }

    /// [`Self::exec`] for background reads that must not hang or flood memory: fails (and
    /// closes the channel) after `timeout`, or once stdout + stderr exceed `max_output` bytes.
    pub async fn exec_bounded(
        &self,
        command: &str,
        timeout: Duration,
        max_output: usize,
    ) -> Result<ExecOutput> {
        let deadline = tokio::time::Instant::now() + timeout;
        self.exec_limited(command, Some(deadline), max_output).await
    }

    async fn exec_limited(
        &self,
        command: &str,
        deadline: Option<tokio::time::Instant>,
        max_output: usize,
    ) -> Result<ExecOutput> {
        let ch = until(deadline, self.handle.channel_open_session())
            .await?
            .map_err(net)?;
        let (mut read, write) = ch.split();
        let result = async {
            until(deadline, write.exec(true, command))
                .await?
                .map_err(net)?;
            let mut out = ExecOutput::default();
            while let Some(msg) = until(deadline, read.wait()).await? {
                match msg {
                    ChannelMsg::Data { data } => out.stdout.extend_from_slice(&data),
                    ChannelMsg::ExtendedData { data, ext: 1 } => {
                        out.stderr.extend_from_slice(&data)
                    }
                    ChannelMsg::ExitStatus { exit_status } => out.exit_status = Some(exit_status),
                    ChannelMsg::Failure => {
                        return Err(Error::Denied("exec request refused".into()))
                    }
                    ChannelMsg::Close => break,
                    _ => {}
                }
                if out.stdout.len() + out.stderr.len() > max_output {
                    return Err(Error::Other("command output too large".into()));
                }
            }
            Ok(out)
        }
        .await;
        if result.is_err() {
            let _ = write.close().await;
        }
        result
    }

    /// Politely disconnect.
    pub async fn disconnect(&self) {
        let _ = self
            .handle
            .disconnect(russh::Disconnect::ByApplication, "bye", "en")
            .await;
    }
}

/// `fut`, failing with a network error once `deadline` (if any) passes.
async fn until<F: std::future::Future>(
    deadline: Option<tokio::time::Instant>,
    fut: F,
) -> Result<F::Output> {
    match deadline {
        Some(d) => tokio::time::timeout_at(d, fut)
            .await
            .map_err(|_| net("command timed out")),
        None => Ok(fut.await),
    }
}

async fn wait_reply(
    read: &mut ChannelReadHalf,
    pending: &mut Vec<Vec<u8>>,
    what: &str,
) -> Result<()> {
    loop {
        match read.wait().await {
            Some(ChannelMsg::Success) => return Ok(()),
            Some(ChannelMsg::Failure) => return Err(Error::Denied(format!("{what} refused"))),
            Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                pending.push(data.to_vec())
            }
            Some(ChannelMsg::Close) | None => {
                return Err(net(format!("channel closed during {what}")))
            }
            Some(_) => {}
        }
    }
}

/// Output of [`SshConnection::exec`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExecOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_status: Option<u32>,
}

/// Interactive shell channel. Implements [`Transport`]; output arrives on the receiver returned
/// alongside it. Keeps its connection alive.
pub struct SshShell {
    write: ChannelWriteHalf<client::Msg>,
    closed: Arc<tokio::sync::Notify>,
    connection: Arc<SshConnection>,
}

impl std::fmt::Debug for SshShell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshShell")
            .field("connection", &self.connection)
            .finish()
    }
}

impl SshShell {
    /// The underlying connection, e.g. for [`SshConnection::exec`].
    pub fn connection(&self) -> &Arc<SshConnection> {
        &self.connection
    }
}

#[async_trait]
impl Transport for SshShell {
    async fn write(&self, data: &[u8]) -> Result<()> {
        self.write.data_bytes(data.to_vec()).await.map_err(net)
    }

    async fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.write
            .window_change(cols as u32, rows as u32, 0, 0)
            .await
            .map_err(net)
    }

    async fn close(&self) -> Result<()> {
        let _ = self.write.eof().await;
        let _ = self.write.close().await;
        self.closed.notify_one();
        Ok(())
    }
}

/// Connect and open the interactive shell described by `params`.
pub async fn connect_shell(
    params: &SshParams,
    verifier: Arc<dyn HostKeyVerifier>,
) -> Result<(SshShell, mpsc::Receiver<TransportEvent>)> {
    let conn = Arc::new(SshConnection::connect(params, verifier).await?);
    conn.open_shell(&params.term, params.cols, params.rows, &params.env)
        .await
}

fn auth_failed(ep: &JumpHop, how: &str) -> Error {
    Error::Auth(format!(
        "{how} authentication failed for {}@{}:{}",
        ep.user, ep.host, ep.port
    ))
}

async fn rsa_hash(h: &Handle<ClientHandler>, alg: &Algorithm) -> Option<HashAlg> {
    if !matches!(alg, Algorithm::Rsa { .. }) {
        return None;
    }
    match h.best_supported_rsa_hash().await {
        Ok(Some(best)) => best,
        // Server did not advertise server-sig-algs: assume a modern server.
        _ => Some(HashAlg::Sha256),
    }
}

async fn authenticate(
    h: &mut Handle<ClientHandler>,
    ep: &JumpHop,
    user_wait: &UserWait,
) -> Result<()> {
    let user = ep.user.clone();
    let ok = match &ep.auth {
        AuthMaterial::Password(pw) => h
            .authenticate_password(user, pw.as_str())
            .await
            .map_err(net)?
            .success(),
        AuthMaterial::Key { pem, passphrase } => {
            let key = russh::keys::decode_secret_key(pem, passphrase.as_ref().map(|p| p.as_str()))
                .map_err(|e| Error::Auth(format!("cannot load private key: {e}")))?;
            let hash = rsa_hash(h, &key.algorithm()).await;
            h.authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                .await
                .map_err(net)?
                .success()
        }
        AuthMaterial::Agent => agent_auth(h, &user).await?,
        AuthMaterial::KeyboardInteractive(handler) => {
            interactive_auth(h, ep, handler.as_ref(), user_wait).await?
        }
    };
    if ok {
        Ok(())
    } else {
        Err(auth_failed(ep, ep.auth.method_name()))
    }
}

const LOGIN_TRIES: usize = 3;
const LOGIN_RETRY: &str = "Permission denied, please try again.";

/// Log in by asking the user, like OpenSSH does: keyboard-interactive (password prompts, OTP
/// codes), falling back to the "password" method for servers that disable keyboard-interactive
/// (Ubuntu's default). Three tries; after a rejected answer the handler is told to try again.
async fn interactive_auth(
    h: &mut Handle<ClientHandler>,
    ep: &JumpHop,
    handler: &dyn KeyboardInteractiveHandler,
    user_wait: &UserWait,
) -> Result<bool> {
    for attempt in 0..LOGIN_TRIES {
        let notice = if attempt == 0 { "" } else { LOGIN_RETRY };
        let mut resp = h
            .authenticate_keyboard_interactive_start(ep.user.clone(), None)
            .await
            .map_err(net)?;
        let mut asked = false;
        let remaining = loop {
            match resp {
                KeyboardInteractiveAuthResponse::Success => return Ok(true),
                KeyboardInteractiveAuthResponse::Failure {
                    remaining_methods, ..
                } => break remaining_methods,
                KeyboardInteractiveAuthResponse::InfoRequest {
                    name,
                    instructions,
                    prompts,
                } => {
                    let prompts: Vec<KiPrompt> = prompts
                        .into_iter()
                        .map(|p| KiPrompt {
                            prompt: p.prompt,
                            echo: p.echo,
                        })
                        .collect();
                    let instructions = match (asked, notice, instructions.is_empty()) {
                        (true, _, _) | (_, "", _) => instructions,
                        (false, n, true) => n.to_string(),
                        (false, n, false) => format!("{n}\n{instructions}"),
                    };
                    asked = true;
                    let Some(answers) = user_wait
                        .around(handler.respond(&name, &instructions, &prompts))
                        .await
                    else {
                        return Err(Error::Auth("login cancelled".into()));
                    };
                    let answers = answers.iter().map(|a| a.to_string()).collect();
                    resp = h
                        .authenticate_keyboard_interactive_respond(answers)
                        .await
                        .map_err(net)?;
                }
            }
        };
        if !asked {
            // Rejected before asking anything: the server does not offer keyboard-interactive.
            if !offers(&remaining, MethodKind::Password) {
                return Ok(false);
            }
            return password_auth(h, ep, handler, user_wait).await;
        }
    }
    Ok(false)
}

/// Ask for a password and use the "password" method.
async fn password_auth(
    h: &mut Handle<ClientHandler>,
    ep: &JumpHop,
    handler: &dyn KeyboardInteractiveHandler,
    user_wait: &UserWait,
) -> Result<bool> {
    let prompt = [KiPrompt {
        prompt: format!("{}@{}'s password: ", ep.user, ep.host),
        echo: false,
    }];
    for attempt in 0..LOGIN_TRIES {
        let notice = if attempt == 0 { "" } else { LOGIN_RETRY };
        let Some(answers) = user_wait.around(handler.respond("", notice, &prompt)).await else {
            return Err(Error::Auth("login cancelled".into()));
        };
        let pw = answers.first().map(|a| a.as_str()).unwrap_or_default();
        match h
            .authenticate_password(ep.user.clone(), pw)
            .await
            .map_err(net)?
        {
            AuthResult::Success => return Ok(true),
            // No point asking again for a password the server will not take.
            AuthResult::Failure {
                remaining_methods, ..
            } if !offers(&remaining_methods, MethodKind::Password) => return Ok(false),
            AuthResult::Failure { .. } => {}
        }
    }
    Ok(false)
}

/// Whether the server's list of methods that can continue includes `m`. An empty list (some
/// servers send none) is taken as "unknown", not "no".
fn offers(methods: &[MethodKind], m: MethodKind) -> bool {
    methods.is_empty() || methods.contains(&m)
}

type DynAgent = AgentClient<Box<dyn AgentStream + Send + Unpin + 'static>>;

async fn connect_agent() -> Result<DynAgent> {
    #[cfg(unix)]
    {
        AgentClient::connect_env()
            .await
            .map(|a| a.dynamic())
            .map_err(|e| Error::Auth(format!("ssh-agent unavailable: {e}")))
    }
    #[cfg(windows)]
    {
        if let Ok(a) = AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await {
            return Ok(a.dynamic());
        }
        AgentClient::connect_pageant()
            .await
            .map(|a| a.dynamic())
            .map_err(|e| Error::Auth(format!("no ssh-agent or Pageant available: {e}")))
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(Error::Auth("ssh-agent unsupported on this platform".into()))
    }
}

async fn agent_auth(h: &mut Handle<ClientHandler>, user: &str) -> Result<bool> {
    let mut agent = connect_agent().await?;
    let ids = agent
        .request_identities()
        .await
        .map_err(|e| Error::Auth(format!("ssh-agent: {e}")))?;
    for id in ids {
        let AgentIdentity::PublicKey { key, .. } = id else {
            continue;
        };
        let hash = rsa_hash(h, &key.algorithm()).await;
        match h
            .authenticate_publickey_with(user, key, hash, &mut agent)
            .await
        {
            Ok(AuthResult::Success) => return Ok(true),
            // Each offer counts against the server's MaxAuthTries: stop once it gives up on keys.
            Ok(AuthResult::Failure {
                remaining_methods, ..
            }) if !offers(&remaining_methods, MethodKind::PublicKey) => break,
            Ok(_) => {}
            Err(AgentAuthError::Send(e)) => return Err(net(e)),
            Err(e) => tracing::debug!("agent key rejected: {e}"),
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::keys::EcdsaCurve;

    #[tokio::test]
    async fn connect_timeout_does_not_count_user_prompts() {
        let ms = Duration::from_millis;
        let user = UserWait::default();
        // 300 ms waiting on the user + 20 ms of work fits a 100 ms limit...
        let slow_user = async {
            user.around(tokio::time::sleep(ms(300))).await;
            tokio::time::sleep(ms(20)).await;
        };
        assert!(timeout_excluding_user(ms(100), &user, slow_user)
            .await
            .is_some());
        // ...300 ms of network does not.
        let user = UserWait::default();
        let slow_net = tokio::time::sleep(ms(300));
        assert!(timeout_excluding_user(ms(100), &user, slow_net)
            .await
            .is_none());
    }

    #[test]
    fn known_host_key_types_come_first() {
        let default = client::Config::default().preferred.key.to_vec();
        assert_eq!(host_key_order(&default, &[]), default);
        let p256 = Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP256,
        };
        let order = host_key_order(&default, std::slice::from_ref(&p256));
        assert_eq!(order[0], p256);
        assert_eq!(order.len(), default.len());
        // A recorded `ssh-rsa` key puts every RSA signature hash first, in russh's order.
        let order = host_key_order(&default, &[Algorithm::Rsa { hash: None }]);
        assert!(order[..3]
            .iter()
            .all(|a| matches!(a, Algorithm::Rsa { .. })));
        assert_eq!(
            order[0],
            Algorithm::Rsa {
                hash: Some(HashAlg::Sha512)
            }
        );
    }
}
