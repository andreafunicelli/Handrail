//! Database tabs (MySQL/MariaDB, PostgreSQL). Each open tab holds a [`DbClient`] (two
//! connections, see `aht-db`) and, for a connection with jump hosts, the SSH tunnel it goes
//! through; both close with the tab.

use crate::state::AppState;
use aht_core::model::{Connection, ConnectionKind, SshAuth};
use aht_core::{ConnectionId, Error, Result};
use aht_db::{ColumnInfo, DbClient, DbKind, DbParams, QueryLimits, QueryOutput, TableInfo};
use aht_ssh::{Forward, SshConnection};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, State};
use tokio::sync::Mutex;
use zeroize::Zeroizing;

/// Error text of `db_open` for a connection without a saved password: the UI asks for one.
pub const PASSWORD_REQUIRED: &str = "password_required";

/// Rows shown per result unless the UI asks for another number; the UI's limit is capped here.
const DEFAULT_ROWS: usize = 1000;
const MAX_ROWS: usize = 100_000;

#[derive(Default)]
pub struct DbHub {
    sessions: Mutex<HashMap<String, Arc<DbSession>>>,
}

struct DbSession {
    client: DbClient,
    kind: DbKind,
    /// The SSH tunnel through the jump hosts; dropped with the session.
    _tunnel: Option<(Arc<SshConnection>, Forward)>,
}

impl DbHub {
    /// Close every database tab (the vault they came from is being deleted or locked).
    pub async fn close_all(&self) {
        let all: Vec<_> = self.sessions.lock().await.drain().map(|(_, s)| s).collect();
        for s in all {
            if let Ok(s) = Arc::try_unwrap(s) {
                s.client.close().await;
            }
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
    if let Some(s) = removed {
        // A statement still running holds another reference; it ends with the connection.
        let _ = s.client.cancel().await;
        if let Ok(s) = Arc::try_unwrap(s) {
            s.client.close().await;
        }
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

/// Stop the statement running in a tab.
#[tauri::command(rename_all = "snake_case")]
pub async fn db_cancel(state: State<'_, AppState>, id: String) -> Result<()> {
    session(&state, &id).await?.client.cancel().await
}
