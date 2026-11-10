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
use aht_db::transfer::{CsvOptions, DumpOptions, ImportOptions, Progress, Summary};
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
    /// Files picked for an import, by the token the page holds (it never names a path).
    picked: std::sync::Mutex<HashMap<String, std::path::PathBuf>>,
    /// Running exports and imports: their tab and the flag that stops them.
    transfers: std::sync::Mutex<HashMap<String, (String, Arc<AtomicBool>)>>,
    /// SQL backups open for reading (no server), by the token the page holds.
    pub(crate) backups: std::sync::Mutex<HashMap<String, Arc<crate::db_backup::OpenBackup>>>,
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
    for (tab, stop) in state.db.transfers.lock().unwrap().values() {
        if *tab == id {
            stop.store(true, Ordering::Relaxed);
        }
    }
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

/// A page of one table's rows (opening it from the schema tree), sorted by a column and
/// filtered by a `WHERE` condition when the user set them.
#[allow(clippy::too_many_arguments)]
#[tauri::command(rename_all = "snake_case")]
pub async fn db_table_rows(
    state: State<'_, AppState>,
    id: String,
    namespace: String,
    table: String,
    max_rows: Option<usize>,
    offset: Option<usize>,
    order_by: Option<String>,
    desc: Option<bool>,
    filter: Option<String>,
) -> Result<QueryOutput> {
    let s = session(&state, &id).await?;
    let l = limits(max_rows);
    let q = aht_db::sql::RowsQuery {
        limit: l.max_rows,
        offset: offset.unwrap_or(0),
        order: order_by.as_deref().map(|c| (c, desc.unwrap_or(false))),
        filter: filter.as_deref(),
    };
    let sql = aht_db::sql::select_rows(s.kind, &namespace, &table, &q);
    s.client.query(&sql, l).await
}

/// The server panel: processes, variables, status or users.
#[tauri::command(rename_all = "snake_case")]
pub async fn db_server_info(
    state: State<'_, AppState>,
    id: String,
    what: aht_db::ServerInfo,
) -> Result<aht_db::ResultSet> {
    session(&state, &id).await?.client.server_info(what).await
}

/// The statements an account change would run, with the password masked (the preview).
#[tauri::command(rename_all = "snake_case")]
pub async fn db_account_preview(
    state: State<'_, AppState>,
    id: String,
    mut change: aht_db::accounts::AccountChange,
) -> Result<Vec<String>> {
    let kind = session(&state, &id).await?.kind;
    let out = aht_db::accounts::account_sql(kind, &change, true, true);
    if let Some(p) = change.password_mut() {
        zeroize::Zeroize::zeroize(p);
    }
    out
}

/// Create, change or drop an account, or grant or revoke privileges.
#[tauri::command(rename_all = "snake_case")]
pub async fn db_account(
    state: State<'_, AppState>,
    id: String,
    mut change: aht_db::accounts::AccountChange,
) -> Result<()> {
    let s = session(&state, &id).await?;
    let out = s.client.account(&change).await;
    if let Some(p) = change.password_mut() {
        zeroize::Zeroize::zeroize(p);
    }
    out
}

/// An account's privileges.
#[tauri::command(rename_all = "snake_case")]
pub async fn db_grants(
    state: State<'_, AppState>,
    id: String,
    user: String,
    host: Option<String>,
) -> Result<aht_db::ResultSet> {
    session(&state, &id)
        .await?
        .client
        .grants(&user, host.as_deref())
        .await
}

/// Stop a server process's statement, or close its connection.
#[tauri::command(rename_all = "snake_case")]
pub async fn db_kill(
    state: State<'_, AppState>,
    id: String,
    pid: u64,
    connection: bool,
) -> Result<()> {
    session(&state, &id)
        .await?
        .client
        .kill(pid, connection)
        .await
}

/// A table's columns, indexes and foreign keys.
#[tauri::command(rename_all = "snake_case")]
pub async fn db_table_structure(
    state: State<'_, AppState>,
    id: String,
    namespace: String,
    table: String,
) -> Result<aht_db::TableStructure> {
    session(&state, &id)
        .await?
        .client
        .structure(&namespace, &table)
        .await
}

/// The statement that creates a table or view.
#[tauri::command(rename_all = "snake_case")]
pub async fn db_table_ddl(
    state: State<'_, AppState>,
    id: String,
    namespace: String,
    table: String,
) -> Result<String> {
    session(&state, &id)
        .await?
        .client
        .ddl(&namespace, &table)
        .await
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

/// Save exported rows (CSV, JSON, SQL) to a file the user picks in a save dialog; `None` when
/// they cancel. The path comes from the dialog, never from the page, so the page cannot write
/// files where it likes.
#[tauri::command(rename_all = "snake_case")]
pub async fn db_export_file(
    app: AppHandle,
    file_name: String,
    contents: String,
) -> Result<Option<String>> {
    use tauri_plugin_dialog::DialogExt;
    // Only a suggested name: no folders.
    let name = std::path::Path::new(&file_name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("export.txt")
        .to_string();
    let ext = std::path::Path::new(&name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("txt")
        .to_string();
    let (tx, rx) = oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(&name)
        .add_filter(ext.to_uppercase(), &[ext.as_str()])
        .save_file(move |picked| {
            let _ = tx.send(picked);
        });
    let Some(picked) = rx
        .await
        .map_err(|_| Error::Other("the save dialog closed".into()))?
    else {
        return Ok(None);
    };
    let path = picked
        .into_path()
        .map_err(|e| Error::Other(e.to_string()))?;
    tokio::fs::write(&path, contents).await?;
    Ok(Some(path.display().to_string()))
}

// ---------- export and import ----------

/// A running transfer: registered so `db_transfer_cancel` (or closing the tab) stops it.
pub(crate) struct Running<'a> {
    hub: &'a DbHub,
    id: String,
    pub(crate) stop: Arc<AtomicBool>,
}

impl<'a> Running<'a> {
    pub(crate) fn new(hub: &'a DbHub, tab: &str, transfer_id: &str) -> Running<'a> {
        let stop = Arc::new(AtomicBool::new(false));
        hub.transfers
            .lock()
            .unwrap()
            .insert(transfer_id.to_string(), (tab.to_string(), stop.clone()));
        Running {
            hub,
            id: transfer_id.to_string(),
            stop,
        }
    }
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.hub.transfers.lock().unwrap().remove(&self.id);
    }
}

/// Progress as `db://transfer` events, at most five a second.
pub(crate) fn transfer_reporter(
    app: &AppHandle,
    transfer_id: &str,
) -> impl FnMut(&Progress) + Send {
    let app = app.clone();
    let id = transfer_id.to_string();
    let mut last: Option<std::time::Instant> = None;
    move |p: &Progress| {
        let now = std::time::Instant::now();
        if last.is_some_and(|l| now.duration_since(l) < Duration::from_millis(200)) {
            return;
        }
        last = Some(now);
        let _ = app.emit("db://transfer", json!({ "transfer_id": id, "progress": p }));
    }
}

#[derive(Serialize)]
pub struct DumpDone {
    pub path: String,
    pub summary: Summary,
}

/// Export a namespace to an SQL file the user picks; `None` when they cancel the dialog.
/// A cancelled or failed export leaves no file behind.
#[tauri::command(rename_all = "snake_case")]
pub async fn db_dump(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    namespace: String,
    options: DumpOptions,
    transfer_id: String,
) -> Result<Option<DumpDone>> {
    use tauri_plugin_dialog::DialogExt;
    let s = session(&state, &id).await?;
    let suggested = format!(
        "{}.sql",
        namespace
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            })
            .collect::<String>()
    );
    let (tx, rx) = oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(&suggested)
        .add_filter("SQL", &["sql"])
        .save_file(move |picked| {
            let _ = tx.send(picked);
        });
    let Some(picked) = rx
        .await
        .map_err(|_| Error::Other("the save dialog closed".into()))?
    else {
        return Ok(None);
    };
    let path = picked
        .into_path()
        .map_err(|e| Error::Other(e.to_string()))?;
    let running = Running::new(&state.db, &id, &transfer_id);
    let mut file = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(&path)?);
    let mut report = transfer_reporter(&app, &transfer_id);
    let done = aht_db::transfer::dump(
        &s.client,
        &namespace,
        &options,
        &mut file,
        &mut report,
        &running.stop,
    )
    .await;
    drop(file);
    match done {
        Ok(summary) if !summary.cancelled => Ok(Some(DumpDone {
            path: path.display().to_string(),
            summary,
        })),
        other => {
            let _ = std::fs::remove_file(&path);
            other.map(|summary| {
                Some(DumpDone {
                    path: String::new(),
                    summary,
                })
            })
        }
    }
}

#[derive(Serialize)]
pub struct PickedFile {
    pub token: String,
    pub name: String,
    pub size: u64,
}

/// Pick a file to import (`sql` or `csv`). The page gets a token for it, not its path.
#[tauri::command(rename_all = "snake_case")]
pub async fn db_pick_file(
    app: AppHandle,
    state: State<'_, AppState>,
    kind: String,
) -> Result<Option<PickedFile>> {
    use tauri_plugin_dialog::DialogExt;
    let (label, exts): (&str, &[&str]) = match kind.as_str() {
        "sql" => ("SQL", &["sql", "txt"]),
        "csv" => ("CSV", &["csv", "tsv", "txt"]),
        _ => return Err(Error::Invalid(format!("unknown file kind {kind}"))),
    };
    let (tx, rx) = oneshot::channel();
    app.dialog()
        .file()
        .add_filter(label, exts)
        .pick_file(move |picked| {
            let _ = tx.send(picked);
        });
    let Some(picked) = rx
        .await
        .map_err(|_| Error::Other("the file dialog closed".into()))?
    else {
        return Ok(None);
    };
    let path = picked
        .into_path()
        .map_err(|e| Error::Other(e.to_string()))?;
    let size = std::fs::metadata(&path)?.len();
    let name = crate::sftp::picked_name(&path)?;
    let token = uuid::Uuid::new_v4().to_string();
    state.db.picked.lock().unwrap().insert(token.clone(), path);
    Ok(Some(PickedFile { token, name, size }))
}

fn picked_path(state: &AppState, token: &str) -> Result<std::path::PathBuf> {
    state
        .db
        .picked
        .lock()
        .unwrap()
        .get(token)
        .cloned()
        .ok_or_else(|| Error::NotFound("the picked file (pick it again)".into()))
}

fn open_picked(state: &AppState, token: &str) -> Result<std::io::BufReader<std::fs::File>> {
    Ok(std::io::BufReader::with_capacity(
        1 << 20,
        std::fs::File::open(picked_path(state, token)?)?,
    ))
}

/// The first records of a picked CSV file.
#[tauri::command(rename_all = "snake_case")]
pub async fn db_csv_preview(
    state: State<'_, AppState>,
    token: String,
    delimiter: char,
) -> Result<Vec<Vec<String>>> {
    aht_db::transfer::csv_preview(open_picked(&state, &token)?, delimiter, 6)
}

/// Run a picked SQL file in `namespace` (or where the file says).
#[tauri::command(rename_all = "snake_case")]
pub async fn db_import_sql(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    token: String,
    namespace: Option<String>,
    options: ImportOptions,
    transfer_id: String,
) -> Result<Summary> {
    let s = session(&state, &id).await?;
    let mut input = open_picked(&state, &token)?;
    let running = Running::new(&state.db, &id, &transfer_id);
    let mut report = transfer_reporter(&app, &transfer_id);
    aht_db::transfer::import_sql(
        &s.client,
        namespace.as_deref(),
        &mut input,
        &options,
        &mut report,
        &running.stop,
    )
    .await
}

/// Insert a picked CSV file's records into a table.
#[allow(clippy::too_many_arguments)]
#[tauri::command(rename_all = "snake_case")]
pub async fn db_import_csv(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    token: String,
    namespace: String,
    table: String,
    options: CsvOptions,
    transfer_id: String,
) -> Result<Summary> {
    let s = session(&state, &id).await?;
    let mut input = open_picked(&state, &token)?;
    let running = Running::new(&state.db, &id, &transfer_id);
    let mut report = transfer_reporter(&app, &transfer_id);
    aht_db::transfer::import_csv(
        &s.client,
        &namespace,
        &table,
        &mut input,
        &options,
        &mut report,
        &running.stop,
    )
    .await
}

/// Stop an export or import (between rows or statements).
#[tauri::command(rename_all = "snake_case")]
pub async fn db_transfer_cancel(state: State<'_, AppState>, transfer_id: String) -> Result<()> {
    if let Some((_, stop)) = state.db.transfers.lock().unwrap().get(&transfer_id) {
        stop.store(true, Ordering::Relaxed);
    }
    Ok(())
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
    let _turn = s.agent_turn.lock().await;
    // The tab may have been closed, unshared or reopened while the user decided or another
    // agent call ran: check again now that this call has its turn.
    let current = shared(&state, &req.database_id).await?;
    if !Arc::ptr_eq(&current, &s) {
        return Err(Error::Denied("the database tab was closed".into()));
    }
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
