//! Port forwards (SSH tunnels) saved on a connection: started with each of its sessions, stopped
//! when the session closes. A reconnect opens a new session, which starts them again.

use crate::state::AppState;
use aht_core::model::{Tunnel, TunnelKind};
use aht_core::{Result, SessionId};
use aht_ssh::{Forward, ForwardSpec, SshConnection};
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

/// How one of a session's tunnels is doing.
#[derive(Clone, Debug, Serialize)]
pub struct TunnelStatus {
    pub tunnel: Tunnel,
    /// The port listened on (the server's for remote forwards), when running.
    pub port: Option<u16>,
    /// Why it is not running. Neither a port nor an error: still starting.
    pub error: Option<String>,
}

fn spec(t: &Tunnel) -> ForwardSpec {
    let bind = (t.bind_host.clone(), t.bind_port);
    let dest = (t.dest_host.clone(), t.dest_port);
    match t.kind {
        TunnelKind::Local => ForwardSpec::Local { bind, dest },
        TunnelKind::Remote => ForwardSpec::Remote { bind, dest },
        TunnelKind::Dynamic => ForwardSpec::Dynamic { bind },
    }
}

/// The enabled tunnels, before they are started.
pub(crate) fn starting(tunnels: &[Tunnel]) -> Vec<TunnelStatus> {
    tunnels
        .iter()
        .filter(|t| t.enabled)
        .map(|t| TunnelStatus {
            tunnel: t.clone(),
            port: None,
            error: None,
        })
        .collect()
}

#[derive(Clone, Serialize)]
struct TunnelsEvent {
    session_id: SessionId,
    tunnels: Vec<TunnelStatus>,
}

/// `session://tunnels`: a session's tunnels started (or failed to).
pub(crate) fn emit(app: &AppHandle, session_id: SessionId, tunnels: Vec<TunnelStatus>) {
    let _ = app.emit(
        "session://tunnels",
        TunnelsEvent {
            session_id,
            tunnels,
        },
    );
}

/// Start the enabled tunnels. One that fails (a port in use, refused by the server) is reported
/// in its status and does not stop the others or the session.
pub(crate) async fn start(
    conn: &Arc<SshConnection>,
    tunnels: &[Tunnel],
) -> (Vec<Forward>, Vec<TunnelStatus>) {
    let mut running = Vec::new();
    let mut status = Vec::new();
    for t in tunnels.iter().filter(|t| t.enabled) {
        match conn.forward(&spec(t)).await {
            Ok(f) => {
                status.push(TunnelStatus {
                    tunnel: t.clone(),
                    port: Some(f.port),
                    error: None,
                });
                running.push(f);
            }
            Err(e) => status.push(TunnelStatus {
                tunnel: t.clone(),
                port: None,
                error: Some(e.to_string()),
            }),
        }
    }
    (running, status)
}

/// The tunnels of a session and how each is doing.
#[tauri::command(rename_all = "snake_case")]
pub async fn session_tunnels(
    state: State<'_, AppState>,
    session_id: SessionId,
) -> Result<Vec<TunnelStatus>> {
    state.terminal.tunnels(session_id).await
}
