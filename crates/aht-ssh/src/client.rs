//! SSH client: connect (with ProxyJump), authenticate, open PTY shells and exec channels.

use crate::auth::{AuthMaterial, KeyboardInteractiveHandler, KiPrompt};
use crate::forward::{connect_forwarded, RemoteForwards};
use crate::verify::{HostKeyDecision, HostKeyVerifier};
use aht_core::terminal::{Transport, TransportEvent};
use aht_core::{Error, Result};
use async_trait::async_trait;
use russh::client::{
    self, ChannelOpenHandle, DisconnectReason, Handle, KeyboardInteractiveAuthResponse, Msg,
    Session,
};
use russh::keys::agent::client::{AgentClient, AgentStream};
use russh::keys::agent::AgentIdentity;
use russh::keys::{Algorithm, HashAlg, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate};
use russh::{Channel, ChannelMsg, ChannelReadHalf, ChannelWriteHalf};
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
    /// Timeout per hop for TCP connect + handshake + auth.
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

pub(crate) struct ClientHandler {
    host: String,
    port: u16,
    verifier: Arc<dyn HostKeyVerifier>,
    decision: SharedSlot<HostKeyDecision>,
    lost: SharedSlot<String>,
    remote: RemoteForwards,
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
        let d = self.verifier.verify(&self.host, self.port, &key).await;
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
        let config = Arc::new(client::Config {
            keepalive_interval: params.keepalive,
            keepalive_max: 3,
            nodelay: true,
            ..Default::default()
        });
        let mut endpoints = params.jump.clone();
        endpoints.push(params.target());

        let mut handles: Vec<Handle<ClientHandler>> = Vec::new();
        let mut last_lost = None;
        let remote = RemoteForwards::default();
        for ep in &endpoints {
            let decision: SharedSlot<HostKeyDecision> = Default::default();
            let lost: SharedSlot<String> = Default::default();
            let handler = ClientHandler {
                host: ep.host.clone(),
                port: ep.port,
                verifier: verifier.clone(),
                decision: decision.clone(),
                lost: lost.clone(),
                remote: remote.clone(),
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
                authenticate(&mut h, ep).await?;
                Ok::<_, Error>(h)
            };
            let h = tokio::time::timeout(params.connect_timeout, attempt)
                .await
                .map_err(|_| net(format!("{}:{}: connect timed out", ep.host, ep.port)))??;
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
        let ch = self.handle.channel_open_session().await.map_err(net)?;
        let (mut read, write) = ch.split();
        write.exec(true, command).await.map_err(net)?;
        let mut out = ExecOutput::default();
        while let Some(msg) = read.wait().await {
            match msg {
                ChannelMsg::Data { data } => out.stdout.extend_from_slice(&data),
                ChannelMsg::ExtendedData { data, ext: 1 } => out.stderr.extend_from_slice(&data),
                ChannelMsg::ExitStatus { exit_status } => out.exit_status = Some(exit_status),
                ChannelMsg::Failure => return Err(Error::Denied("exec request refused".into())),
                ChannelMsg::Close => break,
                _ => {}
            }
        }
        Ok(out)
    }

    /// Politely disconnect.
    pub async fn disconnect(&self) {
        let _ = self
            .handle
            .disconnect(russh::Disconnect::ByApplication, "bye", "en")
            .await;
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

async fn authenticate(h: &mut Handle<ClientHandler>, ep: &JumpHop) -> Result<()> {
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
            interactive_auth(h, ep, handler.as_ref()).await?
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
) -> Result<bool> {
    for attempt in 0..LOGIN_TRIES {
        let notice = if attempt == 0 { "" } else { LOGIN_RETRY };
        let mut resp = h
            .authenticate_keyboard_interactive_start(ep.user.clone(), None)
            .await
            .map_err(net)?;
        let mut asked = false;
        loop {
            match resp {
                KeyboardInteractiveAuthResponse::Success => return Ok(true),
                KeyboardInteractiveAuthResponse::Failure { .. } => break,
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
                    let Some(answers) = handler.respond(&name, &instructions, &prompts).await
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
        }
        if !asked {
            // Rejected before asking anything: the server does not offer keyboard-interactive.
            return password_auth(h, ep, handler).await;
        }
    }
    Ok(false)
}

/// Ask for a password and use the "password" method.
async fn password_auth(
    h: &mut Handle<ClientHandler>,
    ep: &JumpHop,
    handler: &dyn KeyboardInteractiveHandler,
) -> Result<bool> {
    let prompt = [KiPrompt {
        prompt: format!("{}@{}'s password: ", ep.user, ep.host),
        echo: false,
    }];
    for attempt in 0..LOGIN_TRIES {
        let notice = if attempt == 0 { "" } else { LOGIN_RETRY };
        let Some(answers) = handler.respond("", notice, &prompt).await else {
            return Err(Error::Auth("login cancelled".into()));
        };
        let pw = answers.first().map(|a| a.as_str()).unwrap_or_default();
        if h.authenticate_password(ep.user.clone(), pw)
            .await
            .map_err(net)?
            .success()
        {
            return Ok(true);
        }
    }
    Ok(false)
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
            Ok(r) if r.success() => return Ok(true),
            Ok(_) => {}
            Err(e) => tracing::debug!("agent key rejected: {e}"),
        }
    }
    Ok(false)
}
