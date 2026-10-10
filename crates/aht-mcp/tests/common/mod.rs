//! `MockTools`: canned `TerminalTools` that records every call (actor id + tool + args).
#![allow(dead_code)]

use aht_core::model::{ApprovalMode, ConnectionKind, Environment};
use aht_core::terminal::*;
use aht_core::{ConnectionId, Error, OutputId, Result, SessionId};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use uuid::Uuid;

pub fn conn_id() -> ConnectionId {
    ConnectionId(Uuid::from_u128(0xC0))
}
pub fn sess_id() -> SessionId {
    SessionId(Uuid::from_u128(0x5E))
}
pub fn out_id() -> OutputId {
    OutputId(Uuid::from_u128(0x0D))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub actor: String,
    pub tool: &'static str,
    pub args: Value,
}

#[derive(Default)]
pub struct MockTools {
    pub calls: Mutex<Vec<Call>>,
    /// Set when a `run_command("hang")` future is dropped (i.e. the server stopped waiting).
    pub hang_dropped: Arc<AtomicBool>,
}

impl MockTools {
    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }
    fn record(&self, actor: &Actor, tool: &'static str, args: Value) {
        self.calls.lock().unwrap().push(Call {
            actor: actor.id.clone(),
            tool,
            args,
        });
    }
}

pub fn connection() -> ConnectionSummary {
    ConnectionSummary {
        id: conn_id(),
        name: "web-1".into(),
        kind: ConnectionKind::Ssh,
        host: "10.0.0.1".into(),
        user: "deploy".into(),
        env: Environment::Prod,
        group_id: None,
        tags: vec!["web".into()],
    }
}

pub fn session() -> SessionInfo {
    SessionInfo {
        session_id: sess_id(),
        connection_id: Some(conn_id()),
        title: "web-1".into(),
        env: Environment::Prod,
        os: Some("linux".into()),
        shell: Some("bash".into()),
        cwd: Some("/home/deploy".into()),
        user: Some("deploy".into()),
        host: Some("web-1".into()),
        integration: IntegrationMode::Full,
        state: SessionState::Idle,
        approval_mode: ApprovalMode::ConfirmAll,
    }
}

pub fn command_result(command: &str) -> CommandResult {
    CommandResult {
        state: CommandState::Completed,
        exit_code: Some(0),
        output: "x".repeat(2150),
        truncated: false,
        output_id: Some(out_id()),
        duration_ms: 132,
        command: command.into(),
        note: None,
    }
}

struct DropFlag(Arc<AtomicBool>);
impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn check_session(s: SessionId) -> Result<()> {
    if s == sess_id() {
        Ok(())
    } else {
        Err(Error::NotFound(format!("session {s}")))
    }
}

#[async_trait]
impl TerminalTools for MockTools {
    async fn list_connections(&self, actor: &Actor) -> Result<Vec<ConnectionSummary>> {
        self.record(actor, "list_connections", json!({}));
        Ok(vec![connection()])
    }
    async fn open_session(&self, actor: &Actor, connection: ConnectionId) -> Result<SessionInfo> {
        self.record(
            actor,
            "open_session",
            json!({ "connection_id": connection }),
        );
        if connection != conn_id() {
            return Err(Error::NotFound(format!("connection {connection}")));
        }
        Ok(session())
    }
    async fn list_sessions(&self, actor: &Actor) -> Result<Vec<SessionInfo>> {
        self.record(actor, "list_sessions", json!({}));
        Ok(vec![session()])
    }
    async fn session_info(&self, actor: &Actor, s: SessionId) -> Result<SessionInfo> {
        self.record(actor, "session_info", json!({ "session_id": s }));
        check_session(s)?;
        Ok(session())
    }
    async fn run_command(&self, actor: &Actor, req: RunCommandRequest) -> Result<CommandResult> {
        self.record(actor, "run_command", serde_json::to_value(&req).unwrap());
        check_session(req.session_id)?;
        match req.command.as_str() {
            "slow" => {
                tokio::time::sleep(Duration::from_secs(3)).await;
                Ok(command_result("slow"))
            }
            "hang" => {
                let _flag = DropFlag(self.hang_dropped.clone());
                std::future::pending::<()>().await;
                unreachable!()
            }
            "busy" => Err(Error::Busy("an agent command is already running".into())),
            "sudo -i" => Ok(CommandResult {
                state: CommandState::NeedsUser,
                exit_code: None,
                output: "[sudo] password for deploy: ".into(),
                truncated: false,
                output_id: None,
                duration_ms: 40,
                command: req.command.clone(),
                note: Some("password prompt".into()),
            }),
            c => Ok(command_result(c)),
        }
    }
    async fn read_output(
        &self,
        actor: &Actor,
        output: OutputId,
        offset: usize,
        limit: usize,
    ) -> Result<OutputChunk> {
        self.record(
            actor,
            "read_output",
            json!({ "output_id": output, "offset": offset, "limit": limit }),
        );
        Ok(OutputChunk {
            output_id: output,
            offset,
            data: "hello".into(),
            total_len: offset + 5,
            complete: true,
        })
    }
    async fn send_keys(&self, actor: &Actor, s: SessionId, keys: Vec<ControlKey>) -> Result<()> {
        self.record(actor, "send_keys", json!({ "session_id": s, "keys": keys }));
        check_session(s)
    }
}
