//! Server load for the bar under an SSH terminal: read on the session's connection (its own
//! channel, so nothing appears in the terminal).

use crate::state::AppState;
use aht_core::{Result, SessionId};
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
pub struct ServerStats {
    cpu: Option<f32>,
    cores: Option<u32>,
    mem_used: Option<u64>,
    mem_total: Option<u64>,
    disk_used: Option<u64>,
    disk_total: Option<u64>,
    load1: Option<f32>,
    uptime_s: Option<u64>,
}

#[tauri::command(rename_all = "snake_case")]
pub async fn session_stats(
    state: State<'_, AppState>,
    session_id: SessionId,
) -> Result<ServerStats> {
    let s = state
        .terminal
        .ssh_connection(session_id)
        .await?
        .stats()
        .await?;
    Ok(ServerStats {
        cpu: s.cpu,
        cores: s.cores,
        mem_used: s.mem_used,
        mem_total: s.mem_total,
        disk_used: s.disk_used,
        disk_total: s.disk_total,
        load1: s.load1,
        uptime_s: s.uptime_s,
    })
}
