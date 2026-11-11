//! "Test connection" in the connection editor: log in to an unsaved connection the way a
//! session would, then close everything. Nothing runs on the server: no shell, no command, no
//! SQL beyond what opening the database connection does.
//!
//! Host keys are checked against the known hosts like any connection, but a key seen for the
//! first time is never accepted here (there is no prompt to review it): the test reports it, and
//! the user connects once to review and accept it. A changed key is a mismatch, as always.

use crate::cmd_vault::secret_value;
use crate::state::AppState;
use aht_core::model::{Connection, ConnectionKind, SshAuth};
use aht_core::{Error, Result};
use aht_db::{DbClient, DbKind, DbParams};
use aht_ssh::{
    AuthMaterial, HostKeyPrompt, HostKeyVerifier, JumpHop, KeyboardInteractiveHandler, KiPrompt,
    KnownHosts, PromptingVerifier, SshConnection, SshParams,
};
use async_trait::async_trait;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::State;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use zeroize::Zeroizing;

/// The whole test (every jump host included) ends within this time.
const TEST_TIMEOUT: Duration = Duration::from_secs(15);

/// How a test ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TestStatus {
    /// Connected and logged in.
    Success,
    /// The server answered, but the credentials were not checked (RDP).
    Reachable,
    AuthFailed,
    Unreachable,
    Timeout,
    /// A host key not seen before: not accepted by a test.
    HostKeyUnknown,
    HostKeyMismatch,
    /// The connection asks for its password when connecting and none was typed for the test.
    NeedsPassword,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ConnTestResult {
    pub status: TestStatus,
    /// Details (never a secret): the error, or a note.
    pub message: Option<String>,
    /// `host:port` the unknown host key belongs to (the target or a jump host).
    pub host: Option<String>,
    pub fingerprint: Option<String>,
    pub elapsed_ms: u64,
}

impl ConnTestResult {
    fn new(status: TestStatus, message: Option<String>) -> Self {
        Self {
            status,
            message,
            host: None,
            fingerprint: None,
            elapsed_ms: 0,
        }
    }
}

/// A host key seen for the first time during the test.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownKey {
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
}

/// The test's error as a result. `unknown` is the new host key the test refused, if any: the
/// connection failed because of it.
pub fn classify(e: &Error, unknown: Option<UnknownKey>) -> ConnTestResult {
    if let Some(k) = unknown {
        let mut r = ConnTestResult::new(TestStatus::HostKeyUnknown, None);
        r.host = Some(format!("{}:{}", k.host, k.port));
        r.fingerprint = Some(k.fingerprint);
        return r;
    }
    let (status, message) = match e {
        Error::Auth(m) => (TestStatus::AuthFailed, m.clone()),
        Error::HostKeyMismatch(m) => (TestStatus::HostKeyMismatch, m.clone()),
        Error::Network(m) if is_timeout(m) => (TestStatus::Timeout, m.clone()),
        Error::Network(m) => (TestStatus::Unreachable, m.clone()),
        other => (TestStatus::Error, other.to_string()),
    };
    ConnTestResult::new(status, Some(message))
}

fn is_timeout(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("timed out") || m.contains("timeout")
}

/// Records new host keys and refuses them; refuses changed keys (a mismatch). Nothing is ever
/// written to the known hosts.
#[derive(Default)]
struct RefuseNewKeys {
    unknown: Mutex<Option<UnknownKey>>,
}

impl RefuseNewKeys {
    fn unknown(&self) -> Option<UnknownKey> {
        self.unknown
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

#[async_trait]
impl HostKeyPrompt for RefuseNewKeys {
    async fn confirm_new(&self, host: &str, port: u16, fingerprint: &str) -> bool {
        *self.unknown.lock().unwrap_or_else(|e| e.into_inner()) = Some(UnknownKey {
            host: host.into(),
            port,
            fingerprint: fingerprint.into(),
        });
        false
    }

    async fn confirm_replace(&self, _host: &str, _port: u16, _old: &str, _new: &str) -> bool {
        false
    }
}

/// Answers the login prompts of a connection without saved credentials with the password typed
/// for the test, once: a second round means the server did not accept it.
struct TestLogin {
    password: Zeroizing<String>,
    rounds: AtomicUsize,
    /// The server asked for something else than a password (a visible prompt).
    other: AtomicBool,
}

#[async_trait]
impl KeyboardInteractiveHandler for TestLogin {
    async fn respond(
        &self,
        _name: &str,
        _instructions: &str,
        prompts: &[KiPrompt],
    ) -> Option<Vec<Zeroizing<String>>> {
        if prompts.is_empty() {
            return Some(Vec::new());
        }
        if prompts.iter().any(|p| p.echo) {
            self.other.store(true, Ordering::SeqCst);
            return None;
        }
        if self.rounds.fetch_add(1, Ordering::SeqCst) > 0 {
            return None;
        }
        Some(prompts.iter().map(|_| self.password.clone()).collect())
    }
}

/// Secrets typed in the editor (not saved yet); zeroized when dropped.
#[derive(Default)]
struct Typed {
    password: Option<Zeroizing<String>>,
    key: Option<Zeroizing<String>>,
    passphrase: Option<Zeroizing<String>>,
}

fn typed(s: Option<String>) -> Option<Zeroizing<String>> {
    s.map(Zeroizing::new).filter(|s| !s.is_empty())
}

/// Test `connection` (as edited, not necessarily saved). `password`, `key` and `passphrase` are
/// secrets typed in the editor: they replace the saved ones for this test only. A connection
/// logging in as a saved identity uses the identity's login.
#[tauri::command(rename_all = "snake_case")]
pub async fn connection_test(
    state: State<'_, AppState>,
    connection: Connection,
    password: Option<String>,
    key: Option<String>,
    passphrase: Option<String>,
) -> Result<ConnTestResult> {
    let secrets = Typed {
        password: typed(password),
        key: typed(key),
        passphrase: typed(passphrase),
    };
    let started = Instant::now();
    let keys = Arc::new(RefuseNewKeys::default());
    let store = KnownHosts::with_user_known_hosts(state.paths.data_dir.join("known_hosts"));
    let verifier: Arc<dyn HostKeyVerifier> = Arc::new(PromptingVerifier::new(
        store,
        keys.clone() as Arc<dyn HostKeyPrompt>,
    ));
    // On a timeout the test's future is dropped, and with it every connection it opened.
    let mut result = match tokio::time::timeout(
        TEST_TIMEOUT,
        run(&state, connection, secrets, verifier),
    )
    .await
    {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => classify(&e, keys.unknown()),
        Err(_) => ConnTestResult::new(
            TestStatus::Timeout,
            Some(format!(
                "no answer within {} seconds",
                TEST_TIMEOUT.as_secs()
            )),
        ),
    };
    result.elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    tracing::debug!(status = ?result.status, "connection test");
    Ok(result)
}

async fn run(
    state: &AppState,
    mut c: Connection,
    mut secrets: Typed,
    verifier: Arc<dyn HostKeyVerifier>,
) -> Result<ConnTestResult> {
    if c.identity.is_some() {
        let vault = state.vault.lock().await;
        crate::cmd_vault::apply_identity(&vault, &mut c)?;
        // The identity's login is the one used: what is typed in the form does not apply.
        secrets = Typed::default();
    }
    match c.kind {
        ConnectionKind::Ssh => test_ssh(state, &c, secrets, verifier).await,
        ConnectionKind::Mysql | ConnectionKind::Postgres => {
            test_db(state, &c, secrets, verifier).await
        }
        ConnectionKind::Rdp => test_rdp(state, &c, verifier).await,
        ConnectionKind::Sftp | ConnectionKind::Local => Err(Error::Invalid(format!(
            "{:?} connections cannot be tested",
            c.kind
        ))),
    }
}

async fn test_ssh(
    state: &AppState,
    c: &Connection,
    secrets: Typed,
    verifier: Arc<dyn HostKeyVerifier>,
) -> Result<ConnTestResult> {
    let Typed {
        password,
        key,
        passphrase,
    } = secrets;
    let mut login: Option<Arc<TestLogin>> = None;
    let auth = if let Some(pem) = key {
        AuthMaterial::Key { pem, passphrase }
    } else {
        match (&c.auth, password) {
            (SshAuth::Prompt, Some(password)) => {
                let l = Arc::new(TestLogin {
                    password,
                    rounds: AtomicUsize::new(0),
                    other: AtomicBool::new(false),
                });
                login = Some(l.clone());
                AuthMaterial::KeyboardInteractive(l as Arc<dyn KeyboardInteractiveHandler>)
            }
            (SshAuth::Prompt, None) => {
                return Ok(ConnTestResult::new(TestStatus::NeedsPassword, None))
            }
            (_, Some(password)) => AuthMaterial::Password(password),
            // A saved key with a passphrase typed in the form.
            (SshAuth::Key { key, .. }, None) if passphrase.is_some() => AuthMaterial::Key {
                pem: Zeroizing::new(secret_value(state, *key).await?),
                passphrase,
            },
            (_, None) => crate::terminal::auth_material(state, c, None).await?,
        }
    };
    let mut params = SshParams::new(c.host.clone(), c.port, c.user.clone(), auth);
    for id in &c.jump {
        let j = crate::terminal::load_connection(state, *id).await?;
        params.jump.push(JumpHop {
            host: j.host.clone(),
            port: j.port,
            user: j.user.clone(),
            auth: crate::terminal::auth_material(state, &j, None).await?,
        });
    }
    match SshConnection::connect(&params, verifier).await {
        Ok(conn) => {
            // Jump connections close with it.
            conn.disconnect().await;
            Ok(ConnTestResult::new(TestStatus::Success, None))
        }
        Err(Error::Auth(m)) => Err(Error::Auth(match &login {
            Some(l) if l.other.load(Ordering::SeqCst) => {
                "the server asks for more than a password (such as a one-time code): connect \
                 to log in"
                    .into()
            }
            Some(l) if l.rounds.load(Ordering::SeqCst) > 1 => {
                "the server did not accept the password".into()
            }
            _ => m,
        })),
        Err(e) => Err(e),
    }
}

async fn test_db(
    state: &AppState,
    c: &Connection,
    secrets: Typed,
    verifier: Arc<dyn HostKeyVerifier>,
) -> Result<ConnTestResult> {
    let kind = match c.kind {
        ConnectionKind::Mysql => DbKind::Mysql,
        _ => DbKind::Postgres,
    };
    let password = match (secrets.password, &c.auth) {
        (Some(p), _) => p,
        (None, SshAuth::Password { secret }) => Zeroizing::new(secret_value(state, *secret).await?),
        (None, _) => return Ok(ConnTestResult::new(TestStatus::NeedsPassword, None)),
    };
    let tunnel = if c.jump.is_empty() {
        None
    } else {
        Some(crate::rdp::tunnel_with(state, c, verifier).await?)
    };
    let (host, port) = match &tunnel {
        Some((_, f)) => ("127.0.0.1".to_string(), f.port),
        None => (c.host.clone(), c.port),
    };
    // As `db_open` builds them.
    let params = DbParams {
        kind,
        host,
        port,
        user: c.user.clone(),
        password,
        database: c.database.clone().filter(|d| !d.trim().is_empty()),
        tls: c.db_tls.unwrap_or_default(),
        verify_host: tunnel.is_none(),
        connect_timeout: Duration::from_secs(12),
    };
    let outcome = match DbClient::connect(&params).await {
        Ok(client) => {
            client.close().await;
            Ok(ConnTestResult::new(TestStatus::Success, None))
        }
        Err(e) => Err(e),
    };
    if let Some((ssh, forward)) = tunnel {
        drop(forward);
        ssh.disconnect().await;
    }
    outcome
}

/// An X.224 Connection Request with an RDP negotiation request (TLS and CredSSP), the first
/// message of every RDP connection (MS-RDPBCGR 2.2.1.1). An RDP server answers it with a
/// Connection Confirm, which starts with a TPKT header (version 3).
const X224_REQUEST: [u8; 19] = [
    0x03, 0x00, 0x00, 0x13, // TPKT: version 3, length 19
    0x0e, 0xe0, 0x00, 0x00, 0x00, 0x00, 0x00, // X.224 CR: length 14, no references, class 0
    0x01, 0x00, 0x08, 0x00, 0x03, 0x00, 0x00, 0x00, // RDP_NEG_REQ: SSL | HYBRID
];

/// Logging in to RDP needs the helper's whole session, so the test only checks that an RDP
/// server answers at the address (through the jump hosts when there are some).
async fn test_rdp(
    state: &AppState,
    c: &Connection,
    verifier: Arc<dyn HostKeyVerifier>,
) -> Result<ConnTestResult> {
    let tunnel = if c.jump.is_empty() {
        None
    } else {
        Some(crate::rdp::tunnel_with(state, c, verifier).await?)
    };
    let (host, port) = match &tunnel {
        Some((_, f)) => ("127.0.0.1".to_string(), f.port),
        None => (c.host.clone(), c.port),
    };
    let outcome = rdp_probe(&host, port, &format!("{}:{}", c.host, c.port)).await;
    if let Some((ssh, forward)) = tunnel {
        drop(forward);
        ssh.disconnect().await;
    }
    outcome.map(|()| {
        ConnTestResult::new(
            TestStatus::Reachable,
            Some("credentials not checked".into()),
        )
    })
}

async fn rdp_probe(host: &str, port: u16, label: &str) -> Result<()> {
    let mut tcp = tokio::net::TcpStream::connect((host, port))
        .await
        .map_err(|e| Error::Network(format!("{label}: {e}")))?;
    tcp.write_all(&X224_REQUEST)
        .await
        .map_err(|e| Error::Network(format!("{label}: {e}")))?;
    let mut head = [0u8; 4];
    tcp.read_exact(&mut head)
        .await
        .map_err(|e| Error::Network(format!("{label}: no answer from an RDP server ({e})")))?;
    if head[0] != 0x03 {
        return Err(Error::Other(format!(
            "{label} answered, but not as an RDP server"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_map_to_test_results() {
        let s = |e: Error| classify(&e, None).status;
        assert_eq!(s(Error::Auth("denied".into())), TestStatus::AuthFailed);
        assert_eq!(
            s(Error::HostKeyMismatch("h:22".into())),
            TestStatus::HostKeyMismatch
        );
        assert_eq!(
            s(Error::Network("h:22: connect timed out".into())),
            TestStatus::Timeout
        );
        assert_eq!(
            s(Error::Network(
                "timed out connecting to the database".into()
            )),
            TestStatus::Timeout
        );
        assert_eq!(
            s(Error::Network(
                "h:22: Connection refused (os error 111)".into()
            )),
            TestStatus::Unreachable
        );
        assert_eq!(s(Error::NotFound("secret".into())), TestStatus::Error);
        assert_eq!(s(Error::VaultLocked), TestStatus::Error);
        let r = classify(&Error::Other("boom".into()), None);
        assert_eq!(r.message.as_deref(), Some("boom"));
    }

    #[test]
    fn a_refused_new_host_key_is_reported_with_its_fingerprint() {
        let r = classify(
            &Error::Denied("host key for h:2222 not trusted".into()),
            Some(UnknownKey {
                host: "h".into(),
                port: 2222,
                fingerprint: "SHA256:abc".into(),
            }),
        );
        assert_eq!(r.status, TestStatus::HostKeyUnknown);
        assert_eq!(r.host.as_deref(), Some("h:2222"));
        assert_eq!(r.fingerprint.as_deref(), Some("SHA256:abc"));
        // Without a recorded key, a refused key is just an error.
        assert_eq!(
            classify(&Error::Denied("host key for h:22 not trusted".into()), None).status,
            TestStatus::Error
        );
    }

    #[test]
    fn the_rdp_request_is_well_formed() {
        assert_eq!(X224_REQUEST.len(), usize::from(X224_REQUEST[3]));
        assert_eq!(usize::from(X224_REQUEST[4]), X224_REQUEST.len() - 5);
    }

    #[test]
    fn empty_typed_secrets_are_none() {
        assert!(typed(Some(String::new())).is_none());
        assert!(typed(None).is_none());
        assert_eq!(
            typed(Some("x".into())).as_deref().cloned(),
            Some("x".to_string())
        );
    }
}
