//! Database tabs (MySQL/MariaDB, PostgreSQL). Each open tab holds a [`DbClient`] (two
//! connections, see `aht-db`) and, for a connection with jump hosts, the SSH tunnel it goes
//! through; both close with the tab.
//!
//! Agents reach a tab only after the user shares it (off by default). They get a client of
//! their own, opened on first use, so their SQL never runs inside a transaction the user left
//! open in the editor. What they run is classified ([`aht_db::risk`]) and waits for the user's
//! approval in the tab where [`needs_approval`] says so.

use crate::state::AppState;
use aht_core::agent::ApprovalDecision;
use aht_core::model::{ApprovalMode, Connection, ConnectionKind, Environment, SshAuth};
use aht_core::terminal::{Actor, DatabaseSummary, DbQueryRequest, DbSchemaRequest};
use aht_core::{ConnectionId, Error, Result};
use aht_db::risk::SqlRisk;
use aht_db::{ColumnInfo, DbClient, DbKind, DbParams, QueryLimits, QueryOutput, TableInfo};
use aht_ssh::{Forward, SshConnection};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::{oneshot, Mutex, OnceCell};
use zeroize::Zeroizing;

/// Error text of `db_open` for a connection without a saved password: the UI asks for one.
pub const PASSWORD_REQUIRED: &str = "password_required";

/// Rows shown per result unless the UI asks for another number; the UI's limit is capped here.
const DEFAULT_ROWS: usize = 1000;
const MAX_ROWS: usize = 100_000;

/// Agent SQL waiting for the user: the tab it targets and where the answer goes.
type PendingApprovals = HashMap<String, (String, oneshot::Sender<ApprovalDecision>)>;

#[derive(Default)]
pub struct DbHub {
    sessions: Mutex<HashMap<String, Arc<DbSession>>>,
    approvals: std::sync::Mutex<PendingApprovals>,
}

struct DbSession {
    client: DbClient,
    kind: DbKind,
    /// How to open the agents' client (host and port already through the tunnel).
    params: DbParams,
    name: String,
    host: String,
    env: Environment,
    approval_mode: Option<ApprovalMode>,
    version: String,
    /// The user shared this tab with agents.
    shared: AtomicBool,
    /// The agents' own client, opened on first use.
    agent: OnceCell<DbClient>,
    /// One agent call at a time: each is a transaction of its own on that client.
    agent_turn: Mutex<()>,
    /// The SSH tunnel through the jump hosts; dropped with the session.
    _tunnel: Option<(Arc<SshConnection>, Forward)>,
}

impl DbSession {
    /// Stop what runs and close every connection (once no call holds the session any more).
    async fn shut(self: Arc<Self>) {
        // A running statement holds another reference: stop it, so its connection is
        // released (and closed) when it returns.
        let _ = self.client.cancel().await;
        if let Some(a) = self.agent.get() {
            let _ = a.cancel().await;
        }
        if let Ok(s) = Arc::try_unwrap(self) {
            s.client.close().await;
            if let Some(a) = s.agent.into_inner() {
                a.close().await;
            }
        }
    }
}

impl DbHub {
    /// Close every database tab (the vault they came from is being deleted or locked).
    pub async fn close_all(&self) {
        // Dropping the senders denies the SQL still waiting for approval.
        self.approvals.lock().unwrap().clear();
        let all: Vec<_> = self.sessions.lock().await.drain().map(|(_, s)| s).collect();
        for s in all {
            s.shut().await;
        }
    }
}

/// What the UI needs to know about an opened database.
#[derive(Serialize)]
pub struct DbOpened {
    pub id: String,
    pub kind: DbKind,
    pub version: String,
    /// The database the connection is in (`None` when the server picked none, on MySQL).
    pub database: Option<String>,
}

fn db_kind(conn: &Connection) -> Result<DbKind> {
    match conn.kind {
        ConnectionKind::Mysql => Ok(DbKind::Mysql),
        ConnectionKind::Postgres => Ok(DbKind::Postgres),
        _ => Err(Error::Invalid(format!(
            "{} is not a database connection",
            conn.name
        ))),
    }
}

/// Open a database tab for `connection_id`. `password` is the one typed at the prompt for a
/// connection that saves none.
#[tauri::command(rename_all = "snake_case")]
pub async fn db_open(
    app: AppHandle,
    state: State<'_, AppState>,
    connection_id: ConnectionId,
    password: Option<String>,
) -> Result<DbOpened> {
    let conn = crate::terminal::load_connection(&state, connection_id).await?;
    let kind = db_kind(&conn)?;
    let password = Zeroizing::new(match (&conn.auth, password) {
        (SshAuth::Password { secret }, _) => {
            crate::cmd_vault::secret_value(&state, *secret).await?
        }
        (_, Some(p)) => p,
        (_, None) => return Err(Error::Auth(PASSWORD_REQUIRED.into())),
    });

    let tunnel = if conn.jump.is_empty() {
        None
    } else {
        Some(crate::rdp::tunnel(&app, &state, &conn).await?)
    };
    let (host, port) = match &tunnel {
        Some((_, f)) => ("127.0.0.1".to_string(), f.port),
        None => (conn.host.clone(), conn.port),
    };
    let params = DbParams {
        kind,
        host,
        port,
        user: conn.user.clone(),
        password,
        database: conn.database.clone().filter(|d| !d.trim().is_empty()),
        tls: conn.db_tls.unwrap_or_default(),
        // Through a tunnel the client connects to 127.0.0.1, never the certificate's name.
        verify_host: tunnel.is_none(),
        connect_timeout: Duration::from_secs(20),
    };
    let client = DbClient::connect(&params).await?;
    let version = client.version().await.unwrap_or_default();
    let database = client.current_database().await.unwrap_or(None);
    let id = uuid::Uuid::new_v4().to_string();
    state.db.sessions.lock().await.insert(
        id.clone(),
        Arc::new(DbSession {
            client,
            kind,
            params,
            name: conn.name.clone(),
            host: conn.host.clone(),
            env: conn.env,
            approval_mode: conn.approval_mode,
            version: version.clone(),
            shared: AtomicBool::new(false),
            agent: OnceCell::new(),
            agent_turn: Mutex::new(()),
            _tunnel: tunnel,
        }),
    );
    Ok(DbOpened {
        id,
        kind,
        version,
        database,
    })
}

async fn session(state: &AppState, id: &str) -> Result<Arc<DbSession>> {
    state
        .db
        .sessions
        .lock()
        .await
        .get(id)
        .cloned()
        .ok_or_else(|| Error::NotFound(format!("database tab {id}")))
}

#[tauri::command(rename_all = "snake_case")]
pub async fn db_close(state: State<'_, AppState>, id: String) -> Result<()> {
    let removed = state.db.sessions.lock().await.remove(&id);
    // Agent SQL waiting for this tab is denied.
    state
        .db
        .approvals
        .lock()
        .unwrap()
        .retain(|_, (tab, _)| *tab != id);
    if let Some(s) = removed {
        s.shut().await;
    }
    Ok(())
}

/// Databases on the server (PostgreSQL: the ones a new tab could open).
#[tauri::command(rename_all = "snake_case")]
pub async fn db_databases(state: State<'_, AppState>, id: String) -> Result<Vec<String>> {
    session(&state, &id).await?.client.databases().await
}

/// Where tables live: databases on MySQL, schemas on PostgreSQL.
#[tauri::command(rename_all = "snake_case")]
pub async fn db_namespaces(state: State<'_, AppState>, id: String) -> Result<Vec<String>> {
    session(&state, &id).await?.client.namespaces().await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn db_tables(
    state: State<'_, AppState>,
    id: String,
    namespace: String,
) -> Result<Vec<TableInfo>> {
    session(&state, &id).await?.client.tables(&namespace).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn db_columns(
    state: State<'_, AppState>,
    id: String,
    namespace: String,
    table: String,
) -> Result<Vec<ColumnInfo>> {
    session(&state, &id)
        .await?
        .client
        .columns(&namespace, &table)
        .await
}

fn limits(max_rows: Option<usize>) -> QueryLimits {
    QueryLimits {
        max_rows: max_rows.unwrap_or(DEFAULT_ROWS).clamp(1, MAX_ROWS),
        ..QueryLimits::default()
    }
}

/// Run what the user wrote (one or more statements).
#[tauri::command(rename_all = "snake_case")]
pub async fn db_query(
    state: State<'_, AppState>,
    id: String,
    sql: String,
    max_rows: Option<usize>,
) -> Result<QueryOutput> {
    let s = session(&state, &id).await?;
    s.client.query(&sql, limits(max_rows)).await
}

/// The rows of one table (opening it from the schema tree).
#[tauri::command(rename_all = "snake_case")]
pub async fn db_table_rows(
    state: State<'_, AppState>,
    id: String,
    namespace: String,
    table: String,
    max_rows: Option<usize>,
) -> Result<QueryOutput> {
    let s = session(&state, &id).await?;
    let l = limits(max_rows);
    let sql = aht_db::sql::select_rows(s.kind, &namespace, &table, l.max_rows);
    s.client.query(&sql, l).await
}

/// Save edits made in the grid to one table, in one transaction; returns the rows changed.
#[tauri::command(rename_all = "snake_case")]
pub async fn db_apply_edits(
    state: State<'_, AppState>,
    id: String,
    namespace: String,
    table: String,
    edits: Vec<aht_db::sql::RowEdit>,
) -> Result<u64> {
    let s = session(&state, &id).await?;
    s.client.apply_edits(&namespace, &table, &edits).await
}

/// Stop the statement running in a tab.
#[tauri::command(rename_all = "snake_case")]
pub async fn db_cancel(state: State<'_, AppState>, id: String) -> Result<()> {
    session(&state, &id).await?.client.cancel().await
}

// ---------- agents ----------

/// Share a tab with agents, or stop sharing it.
#[tauri::command(rename_all = "snake_case")]
pub async fn db_share(state: State<'_, AppState>, id: String, shared: bool) -> Result<()> {
    session(&state, &id)
        .await?
        .shared
        .store(shared, Ordering::SeqCst);
    if !shared {
        state
            .db
            .approvals
            .lock()
            .unwrap()
            .retain(|_, (tab, _)| *tab != id);
    }
    Ok(())
}

/// The user's answer to agent SQL shown in a tab.
#[tauri::command(rename_all = "snake_case")]
pub async fn db_approval_decide(
    state: State<'_, AppState>,
    approval_id: String,
    decision: ApprovalDecision,
) -> Result<()> {
    let pending = state.db.approvals.lock().unwrap().remove(&approval_id);
    match pending {
        Some((_, tx)) => {
            let _ = tx.send(decision);
            Ok(())
        }
        None => Err(Error::NotFound("approval already resolved".into())),
    }
}

/// Why agent SQL of `risk` must wait for the user, or `None` when it may run at once: reads
/// run unless every action is confirmed, writes wait unless approvals are off (never on a
/// production connection), destructive statements always wait.
pub(crate) fn needs_approval(
    risk: SqlRisk,
    mode: ApprovalMode,
    env: Environment,
) -> Option<&'static str> {
    match risk {
        SqlRisk::Destructive => Some("destructive statement"),
        SqlRisk::Write if env == Environment::Prod => Some("changes data on a production database"),
        SqlRisk::Write if mode != ApprovalMode::Auto => Some("changes data"),
        SqlRisk::Read if mode == ApprovalMode::ConfirmAll => Some("every action is confirmed"),
        _ => None,
    }
}

/// Shown in the tab, with Approve and Deny.
#[derive(Clone, Serialize)]
struct DbApprovalRequest {
    approval_id: String,
    database_id: String,
    actor_label: String,
    sql: String,
    reason: Option<String>,
    risk: SqlRisk,
    statements: usize,
    policy_note: &'static str,
}

const APPROVAL_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// Removes a pending approval when its wait ends for any reason; the tab then drops its card.
struct PendingGuard<'a> {
    app: &'a AppHandle,
    id: String,
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        // An answer removed it already; a timeout or cancel did not. Either way the tab drops
        // the card.
        self.app
            .state::<AppState>()
            .db
            .approvals
            .lock()
            .unwrap()
            .remove(&self.id);
        let _ = self.app.emit("db://approval-resolved", self.id.clone());
    }
}

async fn ask(app: &AppHandle, req: DbApprovalRequest) -> ApprovalDecision {
    let (tx, rx) = oneshot::channel();
    let id = req.approval_id.clone();
    app.state::<AppState>()
        .db
        .approvals
        .lock()
        .unwrap()
        .insert(id.clone(), (req.database_id.clone(), tx));
    let _guard = PendingGuard { app, id };
    let _ = app.emit("db://approval", req);
    match tokio::time::timeout(APPROVAL_TIMEOUT, rx).await {
        Ok(Ok(d)) => d,
        Ok(Err(_)) => ApprovalDecision::Deny {
            message: Some("the tab was closed or no longer shared".into()),
        },
        Err(_) => ApprovalDecision::Deny {
            message: Some("no answer within 15 minutes".into()),
        },
    }
}

/// A tab the user shared with agents.
async fn shared(state: &AppState, id: &str) -> Result<Arc<DbSession>> {
    let s = session(state, id).await?;
    if !s.shared.load(Ordering::SeqCst) {
        return Err(Error::Denied(format!(
            "database {id} is not shared with agents; ask the user to share it"
        )));
    }
    Ok(s)
}

pub(crate) async fn agent_list(state: &AppState) -> Vec<DatabaseSummary> {
    let sessions: Vec<(String, Arc<DbSession>)> = state
        .db
        .sessions
        .lock()
        .await
        .iter()
        .filter(|(_, s)| s.shared.load(Ordering::SeqCst))
        .map(|(id, s)| (id.clone(), s.clone()))
        .collect();
    let mut out = Vec::new();
    for (id, s) in sessions {
        out.push(DatabaseSummary {
            database_id: id,
            name: s.name.clone(),
            kind: match s.kind {
                DbKind::Mysql => ConnectionKind::Mysql,
                DbKind::Postgres => ConnectionKind::Postgres,
            },
            host: s.host.clone(),
            // The one it opened: asking the main connection would wait for a running query.
            database: s.params.database.clone(),
            env: s.env,
            version: s.version.clone(),
        });
    }
    out
}

pub(crate) async fn agent_schema(state: &AppState, req: DbSchemaRequest) -> Result<Value> {
    let s = shared(state, &req.database_id).await?;
    Ok(match (req.namespace, req.table) {
        (None, _) => json!({ "namespaces": s.client.namespaces().await? }),
        (Some(ns), None) => json!({ "tables": s.client.tables(&ns).await? }),
        (Some(ns), Some(t)) => json!({ "columns": s.client.columns(&ns, &t).await? }),
    })
}

/// Rows per result sent to an agent, and the bytes kept of each result.
const AGENT_DEFAULT_ROWS: usize = 100;
const AGENT_MAX_ROWS: usize = 1000;
const AGENT_MAX_BYTES: usize = 64 * 1024;

pub(crate) async fn agent_query(
    app: &AppHandle,
    actor: &Actor,
    req: DbQueryRequest,
) -> Result<Value> {
    let state = app.state::<AppState>();
    let s = shared(&state, &req.database_id).await?;
    let (mut risk, statements) = aht_db::risk::classify(s.kind, &req.sql);
    if statements == 0 {
        return Err(Error::Invalid("no SQL statement to run".into()));
    }
    if req.write {
        risk = risk.max(SqlRisk::Write);
    }
    let (default_mode, _) = crate::terminal::approval_settings(&state).await;
    let mode = s.approval_mode.unwrap_or(default_mode);
    if let Some(note) = needs_approval(risk, mode, s.env) {
        let decision = ask(
            app,
            DbApprovalRequest {
                approval_id: uuid::Uuid::new_v4().to_string(),
                database_id: req.database_id.clone(),
                actor_label: actor.label.clone(),
                sql: req.sql.clone(),
                reason: req.reason.clone(),
                risk,
                statements,
                policy_note: note,
            },
        )
        .await;
        match decision {
            ApprovalDecision::Approve => {}
            ApprovalDecision::Deny { message: Some(m) } => {
                return Err(Error::Denied(format!("the user denied this SQL: {m}")));
            }
            // The card has no editor: an edited answer cannot come from it.
            ApprovalDecision::Deny { message: None } | ApprovalDecision::ApproveEdited { .. } => {
                return Err(Error::Denied("the user denied this SQL".into()));
            }
        }
    }
    // The tab may have been closed or unshared while the user decided.
    let s = shared(&state, &req.database_id).await?;
    let _turn = s.agent_turn.lock().await;
    let client = s
        .agent
        .get_or_try_init(|| DbClient::connect(&s.params))
        .await?;
    let limits = QueryLimits {
        max_rows: req
            .max_rows
            .unwrap_or(AGENT_DEFAULT_ROWS)
            .clamp(1, AGENT_MAX_ROWS),
        max_bytes: AGENT_MAX_BYTES,
    };
    let out = client
        .run_agent(&req.sql, limits, risk == SqlRisk::Read)
        .await?;
    let mut v = serde_json::to_value(&out).map_err(|e| Error::Other(e.to_string()))?;
    v["risk"] = json!(risk);
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_rules() {
        use ApprovalMode::{Auto, ConfirmAll, ConfirmWrites};
        use Environment::{Dev, Prod, Staging};
        use SqlRisk::{Destructive, Read, Write};
        for mode in [ConfirmAll, ConfirmWrites, Auto] {
            for env in [Prod, Staging, Dev, Environment::None] {
                assert!(needs_approval(Destructive, mode, env).is_some());
                assert_eq!(
                    needs_approval(Write, mode, env).is_some(),
                    mode != Auto || env == Prod,
                    "{mode:?} {env:?}"
                );
                assert_eq!(
                    needs_approval(Read, mode, env).is_some(),
                    mode == ConfirmAll
                );
            }
        }
    }
}
