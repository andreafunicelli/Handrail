//! RDP sessions. Each runs in its own `handrail-rdp` helper process (see `rdp-helper/`), which
//! Handrail feeds the connection details and the user's input over stdin; its stdout (desktop
//! tiles, pointer shapes, errors) goes to the UI untouched through a binary channel.
//!
//! A connection with jump hosts reaches the RDP server through an SSH local forward on
//! 127.0.0.1, opened through the jump chain and closed with the session.

use crate::state::{err, AppState};
use aht_core::model::{Connection, ConnectionKind, SshAuth};
use aht_core::{ConnectionId, Error, Result};
use aht_ssh::{
    Forward, ForwardSpec, JumpHop, KnownHosts, PromptingVerifier, SshConnection, SshParams,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Manager, State};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::Mutex;
use zeroize::Zeroizing;

// Message kinds of the helper protocol (rdp-helper/src/proto.rs).
const START: u8 = 0x01;
const INPUT: u8 = 0x02;
const RESIZE: u8 = 0x03;
const CLOSE: u8 = 0x04;
const ERROR: u8 = 0x13;
const TERMINATED: u8 = 0x14;
/// Input batches larger than this are refused (the helper allows 1 MiB per message).
const MAX_INPUT: usize = 256 * 1024;

#[derive(Default)]
pub struct RdpHub {
    sessions: Mutex<HashMap<String, Arc<RdpSession>>>,
}

struct RdpSession {
    stdin: Mutex<ChildStdin>,
    child: Mutex<Child>,
    /// The SSH tunnel through the jump hosts; dropped with the session.
    _tunnel: Option<(Arc<SshConnection>, Forward)>,
}

impl RdpHub {
    /// Close every session (the vault they came from is being deleted).
    pub async fn close_all(&self) {
        let all: Vec<_> = self.sessions.lock().await.drain().map(|(_, s)| s).collect();
        for s in all {
            s.close().await;
        }
    }
}

impl RdpSession {
    async fn send(&self, kind: u8, payload: &[u8]) -> Result<()> {
        let mut msg = Vec::with_capacity(5 + payload.len());
        msg.extend_from_slice(&u32::try_from(payload.len() + 1).map_err(err)?.to_le_bytes());
        msg.push(kind);
        msg.extend_from_slice(payload);
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(&msg).await?;
        stdin.flush().await?;
        Ok(())
    }

    /// Ask the helper to sign off, then make sure it is gone.
    async fn close(&self) {
        let _ = self.send(CLOSE, b"").await;
        let mut child = self.child.lock().await;
        if tokio::time::timeout(std::time::Duration::from_secs(3), child.wait())
            .await
            .is_err()
        {
            let _ = child.kill().await;
        }
    }
}

/// `DOMAIN\user` or `user@domain` (left as is: NLA takes UPNs) → (user, domain).
fn split_user(user: &str) -> (String, Option<String>) {
    match user.split_once('\\') {
        Some((domain, name)) if !domain.is_empty() && !name.is_empty() => {
            (name.into(), Some(domain.into()))
        }
        _ => (user.into(), None),
    }
}

/// The helper, installed next to the app's executable (Tauri's `externalBin`; `cargo build`
/// copies it into target/ as well).
fn helper_path() -> Result<std::path::PathBuf> {
    let exe = std::env::current_exe()?;
    let dir = exe
        .parent()
        .ok_or_else(|| Error::Other("no executable directory".into()))?;
    let path = dir.join(format!("handrail-rdp{}", std::env::consts::EXE_SUFFIX));
    if !path.is_file()
        || std::fs::metadata(&path)
            .map(|m| m.len() == 0)
            .unwrap_or(true)
    {
        return Err(Error::Other(format!(
            "the RDP component is missing ({}); reinstall Handrail, or for a development build run \
             `node scripts/build-rdp-helper.mjs`",
            path.display()
        )));
    }
    Ok(path)
}

/// An SSH local forward to `conn`'s RDP port through its jump hosts, the last one doing the
/// forwarding. Host keys are checked (and new ones confirmed in the UI) as for terminals.
async fn tunnel(
    app: &AppHandle,
    state: &AppState,
    conn: &Connection,
) -> Result<(Arc<SshConnection>, Forward)> {
    let mut hops = Vec::new();
    for id in &conn.jump {
        let j = crate::terminal::load_connection(state, *id).await?;
        let auth = crate::terminal::auth_material(state, &j, None).await?;
        hops.push(JumpHop {
            host: j.host.clone(),
            port: j.port,
            user: j.user.clone(),
            auth,
        });
    }
    let last = hops
        .pop()
        .ok_or_else(|| Error::Invalid("no jump host".into()))?;
    let mut params = SshParams::new(last.host, last.port, last.user, last.auth);
    params.jump = hops;
    let store = KnownHosts::with_user_known_hosts(state.paths.data_dir.join("known_hosts"));
    let verifier = Arc::new(PromptingVerifier::new(
        store,
        crate::terminal::host_key_prompt(app),
    ));
    let ssh = Arc::new(SshConnection::connect(&params, verifier).await?);
    let forward = ssh
        .forward(&ForwardSpec::Local {
            bind: ("127.0.0.1".into(), 0),
            dest: (conn.host.clone(), conn.port),
        })
        .await?;
    Ok((ssh, forward))
}

#[derive(Deserialize)]
pub struct Size {
    width: u16,
    height: u16,
    /// Percent; 100 = no scaling.
    scale: u32,
}

/// Open an RDP session to `connection_id` at the view's size. Desktop updates arrive on
/// `on_event` as raw messages (`[kind][payload]`, see rdp-helper/src/proto.rs); the returned
/// id addresses `rdp_input`, `rdp_resize` and `rdp_close`.
#[tauri::command(rename_all = "snake_case")]
pub async fn rdp_open(
    app: AppHandle,
    state: State<'_, AppState>,
    connection_id: ConnectionId,
    size: Size,
    on_event: Channel<InvokeResponseBody>,
) -> Result<String> {
    let conn = crate::terminal::load_connection(&state, connection_id).await?;
    if conn.kind != ConnectionKind::Rdp {
        return Err(Error::Invalid(format!(
            "{} is not an RDP connection",
            conn.name
        )));
    }
    let SshAuth::Password { secret } = &conn.auth else {
        return Err(Error::Invalid(format!(
            "{} needs a saved password for RDP",
            conn.name
        )));
    };
    let password = Zeroizing::new(crate::cmd_vault::secret_value(&state, *secret).await?);
    let path = helper_path()?;

    let tunnel = if conn.jump.is_empty() {
        None
    } else {
        Some(tunnel(&app, &state, &conn).await?)
    };
    let (host, port) = match &tunnel {
        Some((_, f)) => ("127.0.0.1".to_string(), f.port),
        None => (conn.host.clone(), conn.port),
    };

    let mut child = Command::new(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| Error::Other(format!("could not start the RDP component: {e}")))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| Error::Other("no stdin".into()))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Other("no stdout".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::Other("no stderr".into()))?;

    let id = uuid::Uuid::new_v4().to_string();
    let session = Arc::new(RdpSession {
        stdin: Mutex::new(stdin),
        child: Mutex::new(child),
        _tunnel: tunnel,
    });
    let (user, domain) = split_user(&conn.user);
    let start = Zeroizing::new(
        serde_json::json!({
            "host": host,
            "port": port,
            "username": user,
            "domain": domain,
            "password": password.as_str(),
            "width": size.width,
            "height": size.height,
            "scale": size.scale,
        })
        .to_string(),
    );
    session.send(START, start.as_bytes()).await?;
    state.rdp.sessions.lock().await.insert(id.clone(), session);

    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::warn!(target: "handrail_rdp", "{line}");
        }
    });
    let (app2, id2) = (app.clone(), id.clone());
    tokio::spawn(async move {
        let mut said_why = false;
        loop {
            let mut len = [0u8; 4];
            if stdout.read_exact(&mut len).await.is_err() {
                break;
            }
            let mut msg = vec![0u8; u32::from_le_bytes(len) as usize];
            if msg.is_empty() || stdout.read_exact(&mut msg).await.is_err() {
                break;
            }
            said_why |= msg[0] == ERROR || msg[0] == TERMINATED;
            if on_event.send(InvokeResponseBody::Raw(msg)).is_err() {
                break;
            }
        }
        if !said_why {
            let mut msg = vec![ERROR];
            msg.extend_from_slice(br#"{"message":"The RDP component stopped unexpectedly."}"#);
            let _ = on_event.send(InvokeResponseBody::Raw(msg));
        }
        let state = app2.state::<AppState>();
        let gone = state.rdp.sessions.lock().await.remove(&id2);
        if let Some(s) = gone {
            s.close().await;
        }
    });
    Ok(id)
}

async fn session(state: &AppState, id: &str) -> Result<Arc<RdpSession>> {
    state
        .rdp
        .sessions
        .lock()
        .await
        .get(id)
        .cloned()
        .ok_or_else(|| Error::NotFound(format!("RDP session {id}")))
}

/// Keyboard and mouse input: a JSON array of events, passed to the helper as is.
#[tauri::command(rename_all = "snake_case")]
pub async fn rdp_input(
    state: State<'_, AppState>,
    id: String,
    events: serde_json::Value,
) -> Result<()> {
    if !events.is_array() {
        return Err(Error::Invalid("input must be a list of events".into()));
    }
    let body = serde_json::to_vec(&events).map_err(err)?;
    if body.len() > MAX_INPUT {
        return Err(Error::Invalid("too much input at once".into()));
    }
    session(&state, &id).await?.send(INPUT, &body).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn rdp_resize(state: State<'_, AppState>, id: String, size: Size) -> Result<()> {
    let body =
        serde_json::json!({ "width": size.width, "height": size.height, "scale": size.scale });
    session(&state, &id)
        .await?
        .send(RESIZE, body.to_string().as_bytes())
        .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn rdp_close(state: State<'_, AppState>, id: String) -> Result<()> {
    let gone = state.rdp.sessions.lock().await.remove(&id);
    if let Some(s) = gone {
        s.close().await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_domain_users() {
        assert_eq!(
            split_user("CORP\\alice"),
            ("alice".into(), Some("CORP".into()))
        );
        assert_eq!(
            split_user("alice@corp.example"),
            ("alice@corp.example".into(), None)
        );
        assert_eq!(split_user("alice"), ("alice".into(), None));
        assert_eq!(split_user("\\alice"), ("\\alice".into(), None));
    }
}
