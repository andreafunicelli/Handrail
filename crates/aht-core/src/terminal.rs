//! The single control surface into terminals. Implemented once (in the app, on top of `aht-term`)
//! and consumed by both the built-in agent and the MCP server, so both see identical tools.

use crate::model::{ApprovalMode, ConnectionKind, Environment};
use crate::{ConnectionId, GroupId, OutputId, Result, SessionId};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// How long an agent action waits for the user's approval.
pub const APPROVAL_TIMEOUT_S: u64 = 15 * 60;
/// Longest `run_command` timeout an agent may ask for (counted after approval).
pub const MAX_COMMAND_TIMEOUT_S: u64 = 600;
/// How long a proposed file edit waits for the user's review.
pub const FILE_EDIT_TIMEOUT_S: u64 = 30 * 60;
/// Longest one tool call can legitimately take (approval wait plus run, or a file review), plus
/// margin. Timeouts outside the session (MCP server, harness CLIs) must be at least this.
pub const TOOL_CALL_TIMEOUT_S: u64 = {
    let run = APPROVAL_TIMEOUT_S + MAX_COMMAND_TIMEOUT_S;
    (if run > FILE_EDIT_TIMEOUT_S {
        run
    } else {
        FILE_EDIT_TIMEOUT_S
    }) + 5 * 60
};

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
            .is_none_or(|s| s.contains(&session))
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
    /// Why the session ended, once it is `Disconnected`.
    #[serde(default)]
    pub ended: Option<SessionEnd>,
}

/// How a session ended: the shell exited (`exit_status`), or the connection was lost (`error`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionEnd {
    pub exit_status: Option<u32>,
    pub error: Option<String>,
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
    /// Show new contents for a file to the user as a diff; resolves once they accept (and the
    /// file is saved) or reject it.
    async fn propose_file_edit(
        &self,
        actor: &Actor,
        req: FileEditRequest,
    ) -> Result<FileEditResult> {
        let _ = (actor, req);
        Err(crate::Error::Invalid(
            "file edits are not available here".into(),
        ))
    }
    /// Database tabs the user shared with agents.
    async fn list_databases(&self, actor: &Actor) -> Result<Vec<DatabaseSummary>> {
        let _ = actor;
        Ok(Vec::new())
    }
    /// Namespaces, a namespace's tables, or a table's columns of a shared database tab.
    async fn db_schema(&self, actor: &Actor, req: DbSchemaRequest) -> Result<Value> {
        let _ = (actor, req);
        Err(crate::Error::Invalid(
            "databases are not available here".into(),
        ))
    }
    /// Run SQL on a shared database tab, after the user's approval where it is needed.
    async fn db_query(&self, actor: &Actor, req: DbQueryRequest) -> Result<Value> {
        let _ = (actor, req);
        Err(crate::Error::Invalid(
            "databases are not available here".into(),
        ))
    }
}

/// A database tab the user shared with agents.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseSummary {
    pub database_id: String,
    /// The saved connection's name.
    pub name: String,
    pub kind: ConnectionKind,
    pub host: String,
    /// The database the tab is in (MySQL: may change with `USE`).
    pub database: Option<String>,
    pub env: Environment,
    pub version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbSchemaRequest {
    pub database_id: String,
    /// Without it: the namespaces (databases on MySQL, schemas on PostgreSQL).
    #[serde(default)]
    pub namespace: Option<String>,
    /// With `namespace`: the table's columns instead of the namespace's tables.
    #[serde(default)]
    pub table: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbQueryRequest {
    pub database_id: String,
    pub sql: String,
    /// Shown to the user on the approval card.
    #[serde(default)]
    pub reason: Option<String>,
    /// Rows kept per result (default 100, at most 1000).
    #[serde(default)]
    pub max_rows: Option<usize>,
    /// Treat the SQL as a write even if it reads (it calls a function that changes data).
    #[serde(default)]
    pub write: bool,
}

/// New contents for a whole text file, proposed by an agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEditRequest {
    pub session_id: SessionId,
    /// Absolute path on the session's server.
    pub path: String,
    /// The complete new file.
    pub content: String,
    /// Why; shown to the user above the diff.
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileEditOutcome {
    /// The user accepted and the file was saved.
    Saved,
    Rejected,
    /// Accepted, but saving failed (see `note`).
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEditResult {
    pub outcome: FileEditOutcome,
    pub path: String,
    /// The user's comment on a rejection, or why saving failed.
    pub note: Option<String>,
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
            description: "Open a new terminal tab connected to a saved connection, after the user approves it. \
Returns the new session.",
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
            description: "Send control keys to a session, e.g. ctrl_c to interrupt a timed-out command, or q to quit a pager. \
enter, up, down, tab and ctrl_d only go to a program your own run_command started, never to the shell \
prompt; to run something at the prompt use run_command.",
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
        ToolSpec {
            name: "propose_file_edit",
            description: "Propose new contents for a text file on the server of an SSH session. The user \
reviews a diff in Handrail's editor, then accepts (the file is saved; as root if their file browser runs as \
root) or rejects it. Give the complete new file, not a patch, and read the current file first (e.g. with \
cat) so nothing else changes. Prefer this over sed, echo or tee for editing files. Returns outcome saved, \
rejected (with the user's note) or failed.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session_id": {"type": "string"},
                    "path": {"type": "string", "description": "Absolute path on the server"},
                    "content": {"type": "string", "description": "The complete new file contents"},
                    "reason": {"type": "string", "description": "Short explanation shown above the diff"}
                },
                "required": ["session_id", "path", "content"]
            }),
        },
        ToolSpec {
            name: "list_databases",
            description: "List the database tabs (MySQL/MariaDB, PostgreSQL) the user opened in Handrail \
and shared with agents. Only these can be queried.",
            input_schema: json!({"type": "object", "properties": {}}),
        },
        ToolSpec {
            name: "db_schema",
            description: "Read the schema of a shared database: without namespace, its namespaces \
(databases on MySQL, schemas on PostgreSQL); with namespace, its tables and views; with namespace and \
table, the table's columns (type, nullability, default, primary key).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "database_id": {"type": "string", "description": "Id from list_databases"},
                    "namespace": {"type": "string"},
                    "table": {"type": "string"}
                },
                "required": ["database_id"]
            }),
        },
        ToolSpec {
            name: "db_query",
            description: "Run SQL (one or more statements) on a shared database. Reads (SELECT, SHOW, \
EXPLAIN, ...) run in a read-only transaction and may run without asking; writes and schema changes wait \
for the user's approval depending on their settings, and destructive statements (DROP, TRUNCATE, DELETE or \
UPDATE without WHERE, ...) always do. Each call is its own unit of work: a transaction left open is rolled \
back at the end. Results are cut to max_rows rows and about 64 KB; select only the columns you need and use \
LIMIT. If the user denies, do not retry the same SQL; ask them.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "database_id": {"type": "string", "description": "Id from list_databases"},
                    "sql": {"type": "string"},
                    "reason": {"type": "string", "description": "Short explanation shown to the user on the approval card"},
                    "max_rows": {"type": "integer", "minimum": 1, "maximum": 1000, "description": "Default 100"},
                    "write": {"type": "boolean", "description": "Set when a read-looking statement changes data (e.g. calls a function that writes), so it runs outside the read-only transaction after approval"}
                },
                "required": ["database_id", "sql"]
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
        "propose_file_edit" => ser(tools.propose_file_edit(actor, parse(args)?).await?),
        "list_databases" => ser(tools.list_databases(actor).await?),
        "db_schema" => tools.db_schema(actor, parse(args)?).await,
        "db_query" => tools.db_query(actor, parse(args)?).await,
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
