//! Integration tests against in-process russh servers on 127.0.0.1.

use aht_core::terminal::{Transport, TransportEvent};
use aht_core::Error;
use aht_ssh::*;
use async_trait::async_trait;
use russh::keys::ssh_key::LineEnding;
use russh::keys::{Algorithm, EcdsaCurve, PrivateKey, PublicKey};
use russh::server::{self, Auth, ChannelOpenHandle, Msg, Response, Session};
use russh::{Channel, ChannelId};
use std::borrow::Cow;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use zeroize::Zeroizing;

const USER: &str = "alice";
const PASSWORD: &str = "s3cret";
const T: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq)]
enum SrvEvent {
    Pty { term: String, cols: u32, rows: u32 },
    Resize { cols: u32, rows: u32 },
    Forward { host: String, port: u32 },
}

#[derive(Clone)]
struct SrvState {
    authorized: Vec<PublicKey>,
    allow_forward: bool,
    events: mpsc::UnboundedSender<SrvEvent>,
}

struct TestServer {
    addr: SocketAddr,
    host_key: PublicKey,
    events: mpsc::UnboundedReceiver<SrvEvent>,
    task: tokio::task::JoinHandle<()>,
}

struct SrvHandler {
    st: SrvState,
    line: Vec<u8>,
    forwarded: std::collections::HashSet<ChannelId>,
}

impl server::Handler for SrvHandler {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if user == USER && password == PASSWORD {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn auth_publickey_offered(
        &mut self,
        _user: &str,
        key: &PublicKey,
    ) -> Result<Auth, Self::Error> {
        self.auth_publickey(USER, key).await
    }

    async fn auth_publickey(&mut self, user: &str, key: &PublicKey) -> Result<Auth, Self::Error> {
        let ok = user == USER
            && self
                .st
                .authorized
                .iter()
                .any(|k| k.key_data() == key.key_data());
        Ok(if ok { Auth::Accept } else { Auth::reject() })
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        user: &str,
        _submethods: &str,
        response: Option<Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        match response {
            None => Ok(Auth::Partial {
                name: Cow::Borrowed("login"),
                instructions: Cow::Borrowed(""),
                prompts: Cow::Owned(vec![(Cow::Borrowed("Password: "), false)]),
            }),
            Some(mut r) => {
                let answer = r.next();
                Ok(
                    if user == USER && answer.as_deref() == Some(PASSWORD.as_bytes()) {
                        Auth::Accept
                    } else {
                        Auth::reject()
                    },
                )
            }
        }
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host: &str,
        port: u32,
        _oa: &str,
        _op: u32,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        let _ = self.st.events.send(SrvEvent::Forward {
            host: host.into(),
            port,
        });
        if !self.st.allow_forward {
            return Ok(()); // dropping `reply` rejects
        }
        let Ok(mut tcp) = tokio::net::TcpStream::connect((host, port as u16)).await else {
            return Ok(());
        };
        self.forwarded.insert(channel.id());
        reply.accept().await;
        tokio::spawn(async move {
            let mut stream = channel.into_stream();
            let _ = tokio::io::copy_bidirectional(&mut stream, &mut tcp).await;
        });
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn pty_request(
        &mut self,
        channel: ChannelId,
        term: &str,
        cols: u32,
        rows: u32,
        _pw: u32,
        _ph: u32,
        _modes: &[(russh::Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let _ = self.st.events.send(SrvEvent::Pty {
            term: term.into(),
            cols,
            rows,
        });
        session.channel_success(channel)?;
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        session.data(channel, b"welcome\r\n$ ".to_vec())?;
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        _channel: ChannelId,
        cols: u32,
        rows: u32,
        _pw: u32,
        _ph: u32,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        let _ = self.st.events.send(SrvEvent::Resize { cols, rows });
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        let cmd = String::from_utf8_lossy(data).into_owned();
        session.data(channel, format!("ran: {cmd}\n").into_bytes())?;
        session.extended_data(channel, 1, b"oops\n".to_vec())?;
        session.exit_status_request(channel, 3)?;
        session.eof(channel)?;
        session.close(channel)?;
        Ok(())
    }

    /// Fake shell: echo input; `exit` ends with status 0; `drop` kills the connection.
    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.forwarded.contains(&channel) {
            return Ok(()); // forwarded bytes are handled by the stream task
        }
        session.data(channel, data.to_vec())?;
        for &b in data {
            if b == b'\r' || b == b'\n' {
                let line = std::mem::take(&mut self.line);
                match line.as_slice() {
                    b"exit" => {
                        session.exit_status_request(channel, 0)?;
                        session.eof(channel)?;
                        session.close(channel)?;
                    }
                    b"drop" => return Err(russh::Error::Disconnect),
                    _ => {}
                }
            } else {
                self.line.push(b);
            }
        }
        Ok(())
    }
}

async fn start_server(authorized: Vec<PublicKey>, allow_forward: bool) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    serve(listener, key, authorized, allow_forward)
}

/// Server with a fresh host key on a fixed port; None while the port is still busy.
async fn start_server_on(port: u16, key: PrivateKey) -> Option<TestServer> {
    let listener = TcpListener::bind(("127.0.0.1", port)).await.ok()?;
    Some(serve(listener, key, vec![], false))
}

fn serve(
    listener: TcpListener,
    key: PrivateKey,
    authorized: Vec<PublicKey>,
    allow_forward: bool,
) -> TestServer {
    let host_key = key.public_key().clone();
    let config = Arc::new(server::Config {
        keys: vec![key],
        auth_rejection_time: Duration::from_millis(10),
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..Default::default()
    });
    let (tx, rx) = mpsc::unbounded_channel();
    let st = SrvState {
        authorized,
        allow_forward,
        events: tx,
    };
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let h = SrvHandler {
                st: st.clone(),
                line: Vec::new(),
                forwarded: Default::default(),
            };
            let config = config.clone();
            tokio::spawn(async move {
                if let Ok(s) = server::run_stream(config, sock, h).await {
                    let _ = s.await;
                }
            });
        }
    });
    TestServer {
        addr,
        host_key,
        events: rx,
        task,
    }
}

/// Records prompts; answers with fixed choices.
struct FakePrompt {
    trust_new: bool,
    replace: bool,
    new_calls: AtomicUsize,
    replace_calls: AtomicUsize,
}

impl FakePrompt {
    fn new(trust_new: bool, replace: bool) -> Arc<Self> {
        Arc::new(Self {
            trust_new,
            replace,
            new_calls: AtomicUsize::new(0),
            replace_calls: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl HostKeyPrompt for FakePrompt {
    async fn confirm_new(&self, _h: &str, _p: u16, fp: &str) -> bool {
        assert!(fp.starts_with("SHA256:"));
        self.new_calls.fetch_add(1, Ordering::SeqCst);
        self.trust_new
    }
    async fn confirm_replace(&self, _h: &str, _p: u16, old: &str, new: &str) -> bool {
        assert_ne!(old, new);
        self.replace_calls.fetch_add(1, Ordering::SeqCst);
        self.replace
    }
}

fn params(addr: SocketAddr, auth: AuthMaterial) -> SshParams {
    let mut p = SshParams::new("127.0.0.1", addr.port(), USER, auth);
    p.connect_timeout = T;
    p.keepalive = Some(Duration::from_secs(5));
    p
}

fn trusting(dir: &tempfile::TempDir) -> Arc<dyn HostKeyVerifier> {
    Arc::new(StrictVerifier {
        store: KnownHosts::new(dir.path().join("known_hosts")),
        accept_new: true,
    })
}

async fn read_until(rx: &mut mpsc::Receiver<TransportEvent>, needle: &str) -> String {
    let mut out = Vec::new();
    tokio::time::timeout(T, async {
        while let Some(ev) = rx.recv().await {
            if let TransportEvent::Data(d) = ev {
                out.extend(d);
                if String::from_utf8_lossy(&out).contains(needle) {
                    return;
                }
            }
        }
        panic!(
            "stream ended before {needle:?}: {:?}",
            String::from_utf8_lossy(&out)
        );
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {needle:?}"));
    String::from_utf8_lossy(&out).into_owned()
}

async fn wait_closed(rx: &mut mpsc::Receiver<TransportEvent>) -> (Option<u32>, Vec<String>) {
    let mut errors = Vec::new();
    tokio::time::timeout(T, async {
        while let Some(ev) = rx.recv().await {
            match ev {
                TransportEvent::Closed { exit_status } => return (exit_status, errors),
                TransportEvent::Error(e) => errors.push(e),
                TransportEvent::Data(_) => {}
            }
        }
        panic!("receiver ended without Closed");
    })
    .await
    .expect("timed out waiting for Closed")
}

#[tokio::test]
async fn password_auth_round_trip_resize_and_exit() {
    let mut srv = start_server(vec![], false).await;
    let dir = tempfile::tempdir().unwrap();
    let mut p = params(srv.addr, AuthMaterial::password(PASSWORD));
    p.cols = 120;
    p.rows = 40;
    let (shell, mut rx) = connect_shell(&p, trusting(&dir)).await.unwrap();
    assert_eq!(
        srv.events.recv().await,
        Some(SrvEvent::Pty {
            term: "xterm-256color".into(),
            cols: 120,
            rows: 40
        })
    );
    read_until(&mut rx, "welcome").await;

    shell.write(b"hello world").await.unwrap();
    read_until(&mut rx, "hello world").await;

    shell.resize(132, 50).await.unwrap();
    let ev = tokio::time::timeout(T, srv.events.recv()).await.unwrap();
    assert_eq!(
        ev,
        Some(SrvEvent::Resize {
            cols: 132,
            rows: 50
        })
    );

    shell.write(b"\rexit\r").await.unwrap();
    let (status, errors) = wait_closed(&mut rx).await;
    assert_eq!(status, Some(0));
    assert!(errors.is_empty(), "{errors:?}");
}

#[tokio::test]
async fn wrong_password_fails_with_auth_error() {
    let srv = start_server(vec![], false).await;
    let dir = tempfile::tempdir().unwrap();
    let p = params(srv.addr, AuthMaterial::password("nope"));
    let err = SshConnection::connect(&p, trusting(&dir))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Auth(_)), "{err:?}");
}

async fn key_auth_works(key: PrivateKey, passphrase: Option<&str>) {
    let srv = start_server(vec![key.public_key().clone()], false).await;
    let dir = tempfile::tempdir().unwrap();
    let pem = match passphrase {
        Some(pp) => key
            .encrypt(&mut rand::rng(), pp)
            .unwrap()
            .to_openssh(LineEnding::LF),
        None => key.to_openssh(LineEnding::LF),
    }
    .unwrap();
    let auth = AuthMaterial::Key {
        pem: Zeroizing::new(pem.to_string()),
        passphrase: passphrase.map(|p| Zeroizing::new(p.to_string())),
    };
    let conn = SshConnection::connect(&params(srv.addr, auth), trusting(&dir))
        .await
        .unwrap();
    let out = conn.exec("uptime").await.unwrap();
    assert_eq!(out.stdout, b"ran: uptime\n");
}

#[tokio::test]
async fn ed25519_key_auth() {
    key_auth_works(
        PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap(),
        None,
    )
    .await;
}

#[tokio::test]
async fn encrypted_ed25519_key_auth() {
    key_auth_works(
        PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap(),
        Some("pass phrase"),
    )
    .await;
}

#[tokio::test]
async fn ecdsa_key_auth() {
    key_auth_works(
        PrivateKey::random(
            &mut rand::rng(),
            Algorithm::Ecdsa {
                curve: EcdsaCurve::NistP256,
            },
        )
        .unwrap(),
        None,
    )
    .await;
}

#[tokio::test]
async fn rsa_key_auth() {
    let key = russh::keys::ssh_key::private::RsaKeypair::random(&mut rand::rng(), 2048).unwrap();
    key_auth_works(PrivateKey::from(key), None).await;
}

#[tokio::test]
async fn unauthorized_key_fails() {
    let srv = start_server(vec![], false).await;
    let dir = tempfile::tempdir().unwrap();
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let auth = AuthMaterial::key(key.to_openssh(LineEnding::LF).unwrap().to_string(), None);
    let err = SshConnection::connect(&params(srv.addr, auth), trusting(&dir))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Auth(_)), "{err:?}");
}

struct KiAnswer(Mutex<Vec<KiPrompt>>);

#[async_trait]
impl KeyboardInteractiveHandler for KiAnswer {
    async fn respond(
        &self,
        _name: &str,
        _instr: &str,
        prompts: &[KiPrompt],
    ) -> Option<Vec<Zeroizing<String>>> {
        self.0.lock().unwrap().extend_from_slice(prompts);
        Some(vec![Zeroizing::new(PASSWORD.to_string())])
    }
}

#[tokio::test]
async fn keyboard_interactive_auth() {
    let srv = start_server(vec![], false).await;
    let dir = tempfile::tempdir().unwrap();
    let ki = Arc::new(KiAnswer(Mutex::new(Vec::new())));
    let auth = AuthMaterial::KeyboardInteractive(ki.clone());
    SshConnection::connect(&params(srv.addr, auth), trusting(&dir))
        .await
        .unwrap();
    let prompts = ki.0.lock().unwrap().clone();
    assert_eq!(
        prompts,
        vec![KiPrompt {
            prompt: "Password: ".into(),
            echo: false
        }]
    );
}

/// Answers wrong first, then right; records the instructions shown each time.
struct KiRetry(Mutex<Vec<String>>);

#[async_trait]
impl KeyboardInteractiveHandler for KiRetry {
    async fn respond(
        &self,
        _name: &str,
        instr: &str,
        _prompts: &[KiPrompt],
    ) -> Option<Vec<Zeroizing<String>>> {
        let mut seen = self.0.lock().unwrap();
        seen.push(instr.to_string());
        let answer = if seen.len() == 1 { "wrong" } else { PASSWORD };
        Some(vec![Zeroizing::new(answer.to_string())])
    }
}

#[tokio::test]
async fn keyboard_interactive_retries_after_a_wrong_answer() {
    let srv = start_server(vec![], false).await;
    let dir = tempfile::tempdir().unwrap();
    let ki = Arc::new(KiRetry(Mutex::new(Vec::new())));
    let auth = AuthMaterial::KeyboardInteractive(ki.clone());
    SshConnection::connect(&params(srv.addr, auth), trusting(&dir))
        .await
        .unwrap();
    assert_eq!(
        *ki.0.lock().unwrap(),
        vec![
            "".to_string(),
            "Permission denied, please try again.".to_string()
        ]
    );
}

#[tokio::test]
async fn host_key_new_known_changed() {
    let srv = start_server(vec![], false).await;
    let dir = tempfile::tempdir().unwrap();
    let store = KnownHosts::new(dir.path().join("kh"));
    let p = params(srv.addr, AuthMaterial::password(PASSWORD));

    // New, declined -> refused, nothing recorded.
    let prompt = FakePrompt::new(false, false);
    let v = Arc::new(PromptingVerifier::new(store.clone(), prompt.clone()));
    let err = SshConnection::connect(&p, v).await.unwrap_err();
    assert!(matches!(err, Error::Denied(_)), "{err:?}");
    assert_eq!(prompt.new_calls.load(Ordering::SeqCst), 1);

    // New, accepted -> recorded.
    let prompt = FakePrompt::new(true, false);
    let v = Arc::new(PromptingVerifier::new(store.clone(), prompt.clone()));
    SshConnection::connect(&p, v).await.unwrap();
    assert_eq!(prompt.new_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        store.check("127.0.0.1", srv.addr.port(), &srv.host_key),
        HostKeyStatus::Known
    );

    // Known -> no prompt.
    let prompt = FakePrompt::new(false, false);
    let v = Arc::new(PromptingVerifier::new(store.clone(), prompt.clone()));
    SshConnection::connect(&p, v).await.unwrap();
    assert_eq!(prompt.new_calls.load(Ordering::SeqCst), 0);

    // Changed: a different server key on the same port.
    drop(srv);
    let port = p.port;
    let srv2 = loop {
        // Re-bind the same port with a new host key.
        let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let s = start_server_on(port, key).await;
        if let Some(s) = s {
            break s;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let prompt = FakePrompt::new(true, false);
    let v = Arc::new(PromptingVerifier::new(store.clone(), prompt.clone()));
    let err = SshConnection::connect(&p, v).await.unwrap_err();
    assert!(matches!(err, Error::HostKeyMismatch(_)), "{err:?}");
    assert_eq!(prompt.replace_calls.load(Ordering::SeqCst), 1);
    assert_eq!(prompt.new_calls.load(Ordering::SeqCst), 0);

    // Strict verifier also refuses a changed key even with accept_new.
    let strict = Arc::new(StrictVerifier {
        store: store.clone(),
        accept_new: true,
    });
    let err = SshConnection::connect(&p, strict).await.unwrap_err();
    assert!(matches!(err, Error::HostKeyMismatch(_)), "{err:?}");

    // Explicit replace -> connects and the new key becomes known.
    let prompt = FakePrompt::new(false, true);
    let v = Arc::new(PromptingVerifier::new(store.clone(), prompt.clone()));
    SshConnection::connect(&p, v).await.unwrap();
    assert_eq!(
        store.check("127.0.0.1", port, &srv2.host_key),
        HostKeyStatus::Known
    );
}

#[tokio::test]
async fn exec_collects_stdout_stderr_exit() {
    let srv = start_server(vec![], false).await;
    let dir = tempfile::tempdir().unwrap();
    let conn = SshConnection::connect(
        &params(srv.addr, AuthMaterial::password(PASSWORD)),
        trusting(&dir),
    )
    .await
    .unwrap();
    let out = conn.exec("uname -a").await.unwrap();
    assert_eq!(out.stdout, b"ran: uname -a\n");
    assert_eq!(out.stderr, b"oops\n");
    assert_eq!(out.exit_status, Some(3));
}

#[tokio::test]
async fn exec_alongside_shell() {
    let srv = start_server(vec![], false).await;
    let dir = tempfile::tempdir().unwrap();
    let (shell, mut rx) = connect_shell(
        &params(srv.addr, AuthMaterial::password(PASSWORD)),
        trusting(&dir),
    )
    .await
    .unwrap();
    read_until(&mut rx, "welcome").await;
    let out = shell.connection().exec("id").await.unwrap();
    assert_eq!(out.stdout, b"ran: id\n");
    shell.write(b"still here").await.unwrap();
    read_until(&mut rx, "still here").await;
}

#[tokio::test]
async fn connection_drop_emits_error_then_closed() {
    let srv = start_server(vec![], false).await;
    let dir = tempfile::tempdir().unwrap();
    let (shell, mut rx) = connect_shell(
        &params(srv.addr, AuthMaterial::password(PASSWORD)),
        trusting(&dir),
    )
    .await
    .unwrap();
    read_until(&mut rx, "welcome").await;
    shell.write(b"drop\r").await.unwrap();
    let (status, errors) = wait_closed(&mut rx).await;
    assert_eq!(status, None);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(shell.connection().is_closed() || shell.connection().disconnect_reason().is_some());
}

#[tokio::test]
async fn local_close_ends_stream() {
    let srv = start_server(vec![], false).await;
    let dir = tempfile::tempdir().unwrap();
    let (shell, mut rx) = connect_shell(
        &params(srv.addr, AuthMaterial::password(PASSWORD)),
        trusting(&dir),
    )
    .await
    .unwrap();
    read_until(&mut rx, "welcome").await;
    shell.close().await.unwrap();
    wait_closed(&mut rx).await;
}

#[tokio::test]
async fn two_hop_proxy_jump() {
    let mut hop1 = start_server(vec![], true).await;
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let hop2 = start_server(vec![], true).await;
    let target = start_server(vec![key.public_key().clone()], false).await;
    let dir = tempfile::tempdir().unwrap();
    let store = KnownHosts::new(dir.path().join("kh"));
    let prompt = FakePrompt::new(true, false);
    let v = Arc::new(PromptingVerifier::new(store.clone(), prompt.clone()));

    let mut p = params(
        target.addr,
        AuthMaterial::key(key.to_openssh(LineEnding::LF).unwrap().to_string(), None),
    );
    for h in [&hop1, &hop2] {
        p.jump.push(JumpHop {
            host: "127.0.0.1".into(),
            port: h.addr.port(),
            user: USER.into(),
            auth: AuthMaterial::password(PASSWORD),
        });
    }
    let (shell, mut rx) = connect_shell(&p, v).await.unwrap();
    read_until(&mut rx, "welcome").await;
    shell.write(b"via jump").await.unwrap();
    read_until(&mut rx, "via jump").await;
    // Every hop's host key was verified separately.
    assert_eq!(prompt.new_calls.load(Ordering::SeqCst), 3);
    for s in [&hop1, &hop2, &target] {
        assert_eq!(
            store.check("127.0.0.1", s.addr.port(), &s.host_key),
            HostKeyStatus::Known
        );
    }
    assert_eq!(
        hop1.events.recv().await,
        Some(SrvEvent::Forward {
            host: "127.0.0.1".into(),
            port: hop2.addr.port() as u32
        })
    );
    let out = shell.connection().exec("hostname").await.unwrap();
    assert_eq!(out.stdout, b"ran: hostname\n");
}

#[tokio::test]
async fn jump_refused_when_forwarding_disabled() {
    let hop = start_server(vec![], false).await;
    let target = start_server(vec![], false).await;
    let dir = tempfile::tempdir().unwrap();
    let mut p = params(target.addr, AuthMaterial::password(PASSWORD));
    p.jump.push(JumpHop {
        host: "127.0.0.1".into(),
        port: hop.addr.port(),
        user: USER.into(),
        auth: AuthMaterial::password(PASSWORD),
    });
    let err = SshConnection::connect(&p, trusting(&dir))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Network(_)), "{err:?}");
}

#[tokio::test]
async fn reconnect_gives_up_on_unreachable() {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    drop(l);
    let dir = tempfile::tempdir().unwrap();
    let policy = ReconnectPolicy {
        initial_delay: Duration::from_millis(5),
        max_delay: Duration::from_millis(10),
        multiplier: 2.0,
        max_attempts: Some(2),
    };
    let err = reconnect(
        &params(addr, AuthMaterial::password(PASSWORD)),
        trusting(&dir),
        &policy,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, Error::Network(_)), "{err:?}");
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
