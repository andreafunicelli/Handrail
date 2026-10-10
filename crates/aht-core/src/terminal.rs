//! The single control surface into terminals. Implemented once (in the app, on top of `aht-term`)
//! and consumed by both the built-in agent and the MCP server, so both see identical tools.

use crate::model::{ApprovalMode, ConnectionKind, Environment};
use crate::{ConnectionId, GroupId, OutputId, Result, SessionId};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Who is acting on a terminal. Shown on approval cards and written to the audit log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    /// Stable id, e.g. "builtin:deepseek-chat" or "harness:claude_code:<profile-id>".
    pub id: String,
    /// Human label, e.g. "Claude Code (DevOps)".
    pub label: String,
    /// Sessions this actor may touch. `None` = any session.
    pub session_scope: Option<Vec<SessionId>>,
}

impl Actor {
    pub fn user() -> Self {
        Self {
            id: "user".into(),
            label: "You".into(),
            session_scope: None,
        }
    }

    pub fn may_access(&self, session: SessionId) -> bool {
        self.session_scope
            .as_ref()
            .map_or(true, |s| s.contains(&session))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConnectionSummary {
    pub id: ConnectionId,
    pub name: String,
    pub kind: ConnectionKind,
    pub host: String,
    pub user: String,
    pub env: Environment,
    pub group_id: Option<GroupId>,
    pub tags: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationMode {
    /// OSC 133 shell integration active: reliable command boundaries and exit codes.
    Full,
    /// Prompt-regex + quiescence fallback: exit codes unknown.
    Limited,
    /// Not yet detected.
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Connecting,
    /// At a shell prompt.
    Idle,
    /// A user-started command is running.
    Busy,
    /// An agent command is running.
    AgentRunning,
    /// Full-screen program (alt screen) active, e.g. vim/top/less.
    FullScreen,
    Reconnecting,
    Disconnected,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub session_id: SessionId,
    pub connection_id: Option<ConnectionId>,
    pub title: String,
    pub env: Environment,
    pub os: Option<String>,
    pub shell: Option<String>,
    pub cwd: Option<String>,
    pub user: Option<String>,
    pub host: Option<String>,
    pub integration: IntegrationMode,
    pub state: SessionState,
    pub approval_mode: ApprovalMode,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCommandRequest {
    pub session_id: SessionId,
    pub command: String,
    /// Seconds before returning `TimedOut` with partial output. Default 60, max 600.
    pub timeout_s: Option<u64>,
    /// Why the agent wants to run this; shown on the approval card.
    pub reason: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandState {
    Completed,
    /// Still running in the terminal; poll with `read_output` or interrupt with `send_keys`.
    TimedOut,
    /// Connection dropped or the user took over.
    Interrupted,
    /// The program wants user input (password prompt, full-screen app). Agent must stop and ask.
    NeedsUser,
    /// The user denied the command.
    Denied,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandResult {
    pub state: CommandState,
    pub exit_code: Option<i32>,
    /// ANSI-stripped output; head+tail when `truncated`.
    pub output: String,
    pub truncated: bool,
    pub output_id: Option<OutputId>,
    pub duration_ms: u64,
    /// Command actually run (may differ from the request if the user edited it on approval).
    pub command: String,
    /// Human-readable note, e.g. why the agent was paused.
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputChunk {
    pub output_id: OutputId,
    pub offset: usize,
    pub data: String,
    pub total_len: usize,
    /// True once the command has finished and `offset + data.len() == total_len`.
    pub complete: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlKey {
    CtrlC,
    CtrlD,
    CtrlZ,
    Enter,
    Esc,
    Q,
    Up,
    Down,
    Left,
    Right,
    Tab,
}

impl ControlKey {
    pub fn bytes(self) -> &'static [u8] {
        match self {
            ControlKey::CtrlC => b"\x03",
            ControlKey::CtrlD => b"\x04",
            ControlKey::CtrlZ => b"\x1a",
            ControlKey::Enter => b"\r",
            ControlKey::Esc => b"\x1b",
            ControlKey::Q => b"q",
            ControlKey::Up => b"\x1b[A",
            ControlKey::Down => b"\x1b[B",
            ControlKey::Right => b"\x1b[C",
            ControlKey::Left => b"\x1b[D",
            ControlKey::Tab => b"\t",
        }
    }
}

#[async_trait]
pub trait TerminalTools: Send + Sync {
    async fn list_connections(&self, actor: &Actor) -> Result<Vec<ConnectionSummary>>;
    async fn open_session(&self, actor: &Actor, connection: ConnectionId) -> Result<SessionInfo>;
    async fn list_sessions(&self, actor: &Actor) -> Result<Vec<SessionInfo>>;
    async fn session_info(&self, actor: &Actor, session: SessionId) -> Result<SessionInfo>;
    async fn run_command(&self, actor: &Actor, req: RunCommandRequest) -> Result<CommandResult>;
    async fn read_output(
        &self,
        actor: &Actor,
        output: OutputId,
        offset: usize,
        limit: usize,
    ) -> Result<OutputChunk>;
    async fn send_keys(
        &self,
        actor: &Actor,
        session: SessionId,
        keys: Vec<ControlKey>,
    ) -> Result<()>;
}

/// Tool name + description + JSON Schema for its input. Shared by MCP and the built-in agent.
#[derive(Clone, Debug, Serialize)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

pub fn tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "list_connections",
            description: "List saved connections (servers) the user has configured. Never includes credentials.",
            input_schema: json!({"type": "object", "properties": {}}),
        },
        ToolSpec {
            name: "list_sessions",
            description: "List open terminal sessions (tabs) with their state, shell and environment.",
            input_schema: json!({"type": "object", "properties": {}}),
        },
        ToolSpec {
            name: "open_session",
            description: "Open a new terminal tab connected to a saved connection. Returns the new session.",
            input_schema: json!({
                "type": "object",
                "properties": {"connection_id": {"type": "string", "description": "Id from list_connections"}},
                "required": ["connection_id"]
            }),
        },
        ToolSpec {
            name: "session_info",
            description: "Get OS, shell, user, current directory, integration mode and state of a session.",
            input_schema: json!({
                "type": "object",
                "properties": {"session_id": {"type": "string"}},
                "required": ["session_id"]
            }),
        },
        ToolSpec {
            name: "run_command",
            description: "Type a shell command into the user's live terminal session and wait for it to finish. \
The user sees it run and may need to approve it first. Returns exit code and output (head+tail if long; \
use read_output with output_id for the rest). If state is needs_user, stop and ask the user. \
Do not run interactive full-screen programs (vim, top, less); prefer non-interactive flags.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session_id": {"type": "string"},
                    "command": {"type": "string", "description": "Single shell command line"},
                    "timeout_s": {"type": "integer", "minimum": 1, "maximum": 600, "description": "Default 60"},
                    "reason": {"type": "string", "description": "Short explanation shown to the user on the approval card"}
                },
                "required": ["session_id", "command"]
            }),
        },
        ToolSpec {
            name: "read_output",
            description: "Read more of a command's full output by output_id, paging with offset/limit (characters).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "output_id": {"type": "string"},
                    "offset": {"type": "integer", "minimum": 0},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 32768}
                },
                "required": ["output_id"]
            }),
        },
        ToolSpec {
            name: "send_keys",
            description: "Send control keys to a session, e.g. ctrl_c to interrupt a timed-out command, or q to quit a pager.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session_id": {"type": "string"},
                    "keys": {
                        "type": "array",
                        "items": {"type": "string", "enum": ["ctrl_c", "ctrl_d", "ctrl_z", "enter", "esc", "q", "up", "down", "left", "right", "tab"]}
                    }
                },
                "required": ["session_id", "keys"]
            }),
        },
    ]
}

/// Dispatch a tool call by name with JSON arguments. Used by both MCP and the built-in agent.
pub async fn dispatch_tool(
    tools: &dyn TerminalTools,
    actor: &Actor,
    name: &str,
    args: Value,
) -> Result<Value> {
    use crate::Error;
    fn parse<T: serde::de::DeserializeOwned>(v: Value) -> Result<T> {
        serde_json::from_value(v).map_err(|e| Error::Invalid(e.to_string()))
    }
    fn ser<T: Serialize>(v: T) -> Result<Value> {
        serde_json::to_value(v).map_err(|e| Error::Other(e.to_string()))
    }
    #[derive(Deserialize)]
    struct ConnArg {
        connection_id: ConnectionId,
    }
    #[derive(Deserialize)]
    struct SessArg {
        session_id: SessionId,
    }
    #[derive(Deserialize)]
    struct ReadArg {
        output_id: OutputId,
        #[serde(default)]
        offset: usize,
        limit: Option<usize>,
    }
    #[derive(Deserialize)]
    struct KeysArg {
        session_id: SessionId,
        keys: Vec<ControlKey>,
    }
    match name {
        "list_connections" => ser(tools.list_connections(actor).await?),
        "list_sessions" => ser(tools.list_sessions(actor).await?),
        "open_session" => {
            let a: ConnArg = parse(args)?;
            ser(tools.open_session(actor, a.connection_id).await?)
        }
        "session_info" => {
            let a: SessArg = parse(args)?;
            ser(tools.session_info(actor, a.session_id).await?)
        }
        "run_command" => ser(tools.run_command(actor, parse(args)?).await?),
        "read_output" => {
            let a: ReadArg = parse(args)?;
            let limit = a.limit.unwrap_or(8192).min(32768);
            ser(tools
                .read_output(actor, a.output_id, a.offset, limit)
                .await?)
        }
        "send_keys" => {
            let a: KeysArg = parse(args)?;
            tools.send_keys(actor, a.session_id, a.keys).await?;
            Ok(json!({"ok": true}))
        }
        other => Err(Error::NotFound(format!("tool {other}"))),
    }
}

/// Byte-level transport for a terminal session (SSH channel or local PTY).
#[async_trait]
pub trait Transport: Send + Sync {
    async fn write(&self, data: &[u8]) -> Result<()>;
    async fn resize(&self, cols: u16, rows: u16) -> Result<()>;
    async fn close(&self) -> Result<()>;
}

/// Events emitted by a transport. Delivered over a `tokio::sync::mpsc` channel by implementors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransportEvent {
    Data(Vec<u8>),
    /// Remote closed; `exit_status` if known.
    Closed {
        exit_status: Option<u32>,
    },
    Error(String),
}
