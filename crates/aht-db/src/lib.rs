//! Database client for Handrail's database tabs: MySQL/MariaDB and PostgreSQL.
//!
//! A [`DbClient`] holds two connections: the *main* one runs the user's statements (so `BEGIN`,
//! `USE`, `SET` and temporary tables carry over from one run to the next, as in any SQL
//! console), and a *side* one reads the schema and cancels a running statement.
//!
//! Statements run with the simple (text) protocol: several statements may be sent at once, and
//! every value arrives as text, so any column type can be shown without a decoder per type.

pub mod accounts;
pub mod backup;
pub mod dump;
mod mysql;
mod postgres;
pub mod risk;
pub mod sql;
pub mod transfer;

pub use aht_core::{Error, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use zeroize::Zeroizing;

/// Which server a connection talks to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DbKind {
    Mysql,
    Postgres,
}

impl DbKind {
    pub fn default_port(self) -> u16 {
        match self {
            DbKind::Mysql => 3306,
            DbKind::Postgres => 5432,
        }
    }
}

/// Transport encryption (the connection model's setting).
pub use aht_core::model::DbTls;

/// Where and as whom to connect.
#[derive(Clone)]
pub struct DbParams {
    pub kind: DbKind,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: Zeroizing<String>,
    /// Database to open; the server's default when empty.
    pub database: Option<String>,
    pub tls: DbTls,
    /// Check the certificate's host name too (`Verify` only). Off through a tunnel, where the
    /// client connects to 127.0.0.1.
    pub verify_host: bool,
    pub connect_timeout: Duration,
}

impl std::fmt::Debug for DbParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbParams")
            .field("kind", &self.kind)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("database", &self.database)
            .field("tls", &self.tls)
            .finish_non_exhaustive()
    }
}

/// One cell of a result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Value {
    Null,
    Text(String),
    /// Bytes that are not UTF-8 (a BLOB), hex-encoded.
    Bytes {
        hex: String,
    },
}

impl Value {
    pub(crate) fn from_bytes(b: &[u8]) -> Value {
        match std::str::from_utf8(b) {
            Ok(s) => Value::Text(s.to_owned()),
            Err(_) => Value::Bytes {
                hex: hex::encode(b),
            },
        }
    }

    fn size(&self) -> usize {
        match self {
            Value::Null => 4,
            Value::Text(s) => s.len(),
            Value::Bytes { hex } => hex.len(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ResultColumn {
    pub name: String,
    /// The server's type name (`VARCHAR`, `int4`...).
    pub type_name: String,
}

/// The outcome of one statement.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ResultSet {
    /// Empty for a statement that returns no rows (and for a query that returned none).
    pub columns: Vec<ResultColumn>,
    pub rows: Vec<Vec<Value>>,
    pub rows_affected: u64,
    /// More rows existed than were kept (see [`QueryLimits`]).
    pub truncated: bool,
}

/// What a run of statements produced: one result per statement that completed, then the error
/// that stopped the run, if any.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct QueryOutput {
    pub results: Vec<ResultSet>,
    pub error: Option<String>,
    pub elapsed_ms: u64,
}

/// How much of a result is kept.
#[derive(Clone, Copy, Debug)]
pub struct QueryLimits {
    /// Rows per result set.
    pub max_rows: usize,
    /// Text kept over all results, in bytes.
    pub max_bytes: usize,
}

impl Default for QueryLimits {
    fn default() -> Self {
        Self {
            max_rows: 1000,
            max_bytes: 32 * 1024 * 1024,
        }
    }
}

/// Collects rows into result sets within [`QueryLimits`].
pub(crate) struct Collector {
    limits: QueryLimits,
    bytes: usize,
    current: ResultSet,
    pub(crate) out: QueryOutput,
}

impl Collector {
    pub(crate) fn new(limits: QueryLimits) -> Self {
        Self {
            limits,
            bytes: 0,
            current: ResultSet::default(),
            out: QueryOutput::default(),
        }
    }

    /// Whether this row is kept; `false` once a limit was reached (the caller then stops the
    /// statement).
    pub(crate) fn row(
        &mut self,
        columns: impl FnOnce() -> Vec<ResultColumn>,
        values: impl FnOnce() -> Vec<Value>,
    ) -> bool {
        if self.current.columns.is_empty() {
            self.current.columns = columns();
        }
        if self.current.truncated
            || self.current.rows.len() >= self.limits.max_rows
            || self.bytes >= self.limits.max_bytes
        {
            self.current.truncated = true;
            return false;
        }
        let values = values();
        self.bytes += values.iter().map(Value::size).sum::<usize>();
        self.current.rows.push(values);
        true
    }

    pub(crate) fn truncated(&self) -> bool {
        self.current.truncated
    }

    /// A statement finished.
    pub(crate) fn done(&mut self, rows_affected: u64) {
        let mut set = std::mem::take(&mut self.current);
        set.rows_affected = rows_affected;
        self.out.results.push(set);
    }

    /// The run stopped with an error. Rows already received for the failed statement are kept.
    pub(crate) fn fail(&mut self, message: String) {
        if !self.current.rows.is_empty() || !self.current.columns.is_empty() {
            let set = std::mem::take(&mut self.current);
            self.out.results.push(set);
        }
        self.out.error = Some(message);
    }

    pub(crate) fn finish(mut self, elapsed: Duration) -> QueryOutput {
        // Rows of a statement whose completion never arrived (cancelled after a cut).
        if !self.current.rows.is_empty() || !self.current.columns.is_empty() {
            let set = std::mem::take(&mut self.current);
            self.out.results.push(set);
        }
        self.out.elapsed_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
        self.out
    }
}

/// A table or view.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct TableInfo {
    pub name: String,
    pub view: bool,
    /// A PostgreSQL materialized view (also `view`): it has its own DROP / RENAME.
    pub materialized: bool,
    /// The server's estimate of the rows (statistics, not a count); `None` for views and
    /// tables never analysed.
    pub rows: Option<i64>,
    /// Data and indexes, in bytes.
    pub size: Option<i64>,
    /// The storage engine (MySQL).
    pub engine: Option<String>,
    pub comment: Option<String>,
}

/// A table's columns, indexes and foreign keys (the structure tab).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct TableStructure {
    pub columns: Vec<ColumnDetail>,
    pub indexes: Vec<IndexInfo>,
    pub foreign_keys: Vec<ForeignKeyInfo>,
}

/// A column with what the structure tab shows beyond [`ColumnInfo`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ColumnDetail {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    /// The default as the server reports it (an expression).
    pub default: Option<String>,
    pub primary_key: bool,
    /// MySQL `AUTO_INCREMENT`, PostgreSQL identity (`ALWAYS` / `BY DEFAULT`), or a generated
    /// column's expression: shown as is.
    pub extra: Option<String>,
    pub comment: Option<String>,
    /// The column's collation (MySQL; restated when the column is changed, so a change does
    /// not reset it to the table's default).
    pub collation: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct IndexInfo {
    pub name: String,
    /// Columns or expressions, in order.
    pub columns: Vec<String>,
    pub unique: bool,
    pub primary: bool,
    /// `BTREE`, `HASH`, `FULLTEXT`, `gin`...
    pub kind: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ForeignKeyInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub ref_namespace: String,
    pub ref_table: String,
    pub ref_columns: Vec<String>,
    /// `CASCADE`, `SET NULL`, `RESTRICT`, `NO ACTION`, `SET DEFAULT`.
    pub on_update: String,
    pub on_delete: String,
}

/// What the server panel shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerInfo {
    /// Connections and what they run.
    Processes,
    /// Settings (global variables / pg_settings).
    Variables,
    /// Counters (global status / pg_stat_database).
    Status,
    /// Accounts (mysql.user / pg_roles).
    Users,
}

/// Rows of text as a result set the grid shows.
pub(crate) fn text_result(columns: &[&str], rows: Vec<Vec<Option<String>>>) -> ResultSet {
    ResultSet {
        columns: columns
            .iter()
            .map(|c| ResultColumn {
                name: (*c).to_string(),
                type_name: "TEXT".into(),
            })
            .collect(),
        rows: rows
            .into_iter()
            .map(|r| {
                r.into_iter()
                    .map(|v| v.map_or(Value::Null, Value::Text))
                    .collect()
            })
            .collect(),
        rows_affected: 0,
        truncated: false,
    }
}

/// A list the server sent joined by the unit separator.
pub(crate) fn split_list(text: Option<&String>) -> Vec<String> {
    text.map(|t| {
        t.split('\u{1f}')
            .map(str::to_string)
            .filter(|s| !s.is_empty())
            .collect()
    })
    .unwrap_or_default()
}

/// A number the server sent as text; negative or missing is `None`.
pub(crate) fn count(text: Option<&String>) -> Option<i64> {
    text.and_then(|t| t.trim().parse::<f64>().ok())
        .filter(|n| *n >= 0.0)
        .map(|n| n as i64)
}

/// Text that is empty means none.
pub(crate) fn non_empty(text: Option<&String>) -> Option<String> {
    text.filter(|t| !t.trim().is_empty()).cloned()
}

/// A column of a table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ColumnInfo {
    pub name: String,
    /// The full type as the server writes it (`int(11) unsigned`, `character varying(80)`).
    pub data_type: String,
    pub nullable: bool,
    pub default: Option<String>,
    pub primary_key: bool,
}

/// A connected database client.
pub struct DbClient {
    inner: Inner,
}

enum Inner {
    Mysql(mysql::Client),
    Postgres(postgres::Client),
}

impl DbClient {
    /// Connect (both connections), failing with the server's message.
    pub async fn connect(params: &DbParams) -> Result<DbClient> {
        let inner = match params.kind {
            DbKind::Mysql => Inner::Mysql(mysql::Client::connect(params).await?),
            DbKind::Postgres => Inner::Postgres(postgres::Client::connect(params).await?),
        };
        Ok(DbClient { inner })
    }

    pub fn kind(&self) -> DbKind {
        match self.inner {
            Inner::Mysql(_) => DbKind::Mysql,
            Inner::Postgres(_) => DbKind::Postgres,
        }
    }

    /// The server's version string.
    pub async fn version(&self) -> Result<String> {
        match &self.inner {
            Inner::Mysql(c) => c.version().await,
            Inner::Postgres(c) => c.version().await,
        }
    }

    /// The database the main connection is in (changes with `USE` on MySQL).
    pub async fn current_database(&self) -> Result<Option<String>> {
        match &self.inner {
            Inner::Mysql(c) => c.current_database().await,
            Inner::Postgres(c) => c.current_database().await,
        }
    }

    /// Databases on the server. On PostgreSQL a connection reaches only its own database;
    /// another one needs a new connection.
    pub async fn databases(&self) -> Result<Vec<String>> {
        match &self.inner {
            Inner::Mysql(c) => c.databases().await,
            Inner::Postgres(c) => c.databases().await,
        }
    }

    /// The namespaces tables live in: databases on MySQL, schemas of the connected database
    /// on PostgreSQL (system schemas left out).
    pub async fn namespaces(&self) -> Result<Vec<String>> {
        match &self.inner {
            Inner::Mysql(c) => c.databases().await,
            Inner::Postgres(c) => c.schemas().await,
        }
    }

    pub async fn tables(&self, namespace: &str) -> Result<Vec<TableInfo>> {
        match &self.inner {
            Inner::Mysql(c) => c.tables(namespace).await,
            Inner::Postgres(c) => c.tables(namespace).await,
        }
    }

    /// The statement that creates a table or view (`SHOW CREATE` on MySQL, rebuilt from the
    /// catalog on PostgreSQL).
    pub async fn ddl(&self, namespace: &str, table: &str) -> Result<String> {
        match &self.inner {
            Inner::Mysql(c) => c.ddl(namespace, table).await,
            Inner::Postgres(c) => c.ddl(namespace, table).await,
        }
    }

    /// The server panel: processes, settings, counters or accounts (read on the side
    /// connection, so a long statement in the editor does not block it).
    pub async fn server_info(&self, what: ServerInfo) -> Result<ResultSet> {
        match &self.inner {
            Inner::Mysql(c) => c.server_info(what).await,
            Inner::Postgres(c) => c.server_info(what).await,
        }
    }

    /// Apply a change to an account (on the side connection). On MySQL the password is quoted
    /// for the session's `sql_mode`.
    pub async fn account(&self, change: &accounts::AccountChange) -> Result<()> {
        match &self.inner {
            Inner::Mysql(c) => {
                let mode = c.sql_mode().await?;
                let escapes = !mode.to_uppercase().contains("NO_BACKSLASH_ESCAPES");
                for st in accounts::account_sql(DbKind::Mysql, change, escapes, false)? {
                    c.side_exec(&st).await?;
                }
                Ok(())
            }
            Inner::Postgres(c) => {
                // One string: PostgreSQL runs it as one transaction (all or nothing).
                let all = accounts::account_sql(DbKind::Postgres, change, true, false)?.join("\n");
                c.side_exec(&all).await
            }
        }
    }

    /// An account's privileges as rows (`SHOW GRANTS` on MySQL; table privileges per schema
    /// on PostgreSQL).
    pub async fn grants(&self, user: &str, host: Option<&str>) -> Result<ResultSet> {
        match &self.inner {
            Inner::Mysql(c) => {
                let mode = c.sql_mode().await?;
                let escapes = !mode.to_uppercase().contains("NO_BACKSLASH_ESCAPES");
                let a = accounts::account(DbKind::Mysql, user, host, escapes)?;
                c.side_text(&format!("SHOW GRANTS FOR {a}"), &["grant"])
                    .await
            }
            Inner::Postgres(c) => c.role_grants(user).await,
        }
    }

    /// Stop what a connection runs (`connection`: close the connection itself).
    pub async fn kill(&self, pid: u64, connection: bool) -> Result<()> {
        match &self.inner {
            Inner::Mysql(c) => c.kill(pid, connection).await,
            Inner::Postgres(c) => c.kill(pid, connection).await,
        }
    }

    /// A table's columns, indexes and foreign keys.
    pub async fn structure(&self, namespace: &str, table: &str) -> Result<TableStructure> {
        match &self.inner {
            Inner::Mysql(c) => c.structure(namespace, table).await,
            Inner::Postgres(c) => c.structure(namespace, table).await,
        }
    }

    pub async fn columns(&self, namespace: &str, table: &str) -> Result<Vec<ColumnInfo>> {
        match &self.inner {
            Inner::Mysql(c) => c.columns(namespace, table).await,
            Inner::Postgres(c) => c.columns(namespace, table).await,
        }
    }

    /// Run `sql` (one or more statements) on the main connection.
    pub async fn query(&self, sql: &str, limits: QueryLimits) -> Result<QueryOutput> {
        match &self.inner {
            Inner::Mysql(c) => c.query(sql, limits).await,
            Inner::Postgres(c) => c.query(sql, limits).await,
        }
    }

    /// Run SQL written by an agent on the main connection. With `read_only` it runs in a
    /// read-only transaction, so a statement that writes fails on the server whatever it looks
    /// like. Either way a transaction left open is rolled back afterwards: one call is one unit
    /// of work, and nothing stays locked between calls.
    pub async fn run_agent(
        &self,
        sql: &str,
        limits: QueryLimits,
        read_only: bool,
    ) -> Result<QueryOutput> {
        // A call cut short (cancelled by the agent) may have left a transaction open; inside
        // it, PostgreSQL's BEGIN READ ONLY would be ignored.
        let _ = self.query("ROLLBACK", limits).await;
        if read_only {
            let begin = match self.kind() {
                DbKind::Mysql => "START TRANSACTION READ ONLY",
                DbKind::Postgres => "BEGIN READ ONLY",
            };
            if let Some(e) = self.query(begin, limits).await?.error {
                return Err(Error::Other(format!(
                    "could not start a read-only transaction: {e}"
                )));
            }
        }
        let out = self.query(sql, limits).await;
        let _ = self.query("ROLLBACK", limits).await;
        out
    }

    /// Apply edits made in the grid to `namespace.table`, all in one transaction; returns the
    /// rows changed. Rows are found by their primary key (see [`sql::edit_statements`]).
    pub async fn apply_edits(
        &self,
        namespace: &str,
        table: &str,
        edits: &[sql::RowEdit],
    ) -> Result<u64> {
        let columns = self.columns(namespace, table).await?;
        let statements = sql::edit_statements(self.kind(), namespace, table, &columns, edits)?;
        if statements.is_empty() {
            return Ok(0);
        }
        match &self.inner {
            Inner::Mysql(c) => c.apply_edits(&statements).await,
            Inner::Postgres(c) => c.apply_edits(&statements).await,
        }
    }

    /// Ask the server to stop the statement running on the main connection, if any.
    pub async fn cancel(&self) -> Result<()> {
        match &self.inner {
            Inner::Mysql(c) => c.cancel().await,
            Inner::Postgres(c) => c.cancel().await,
        }
    }

    /// Close both connections.
    pub async fn close(self) {
        match self.inner {
            Inner::Mysql(c) => c.close().await,
            Inner::Postgres(c) => c.close().await,
        }
    }
}

/// An sqlx error as Handrail's: authentication and network problems keep their kind, a server
/// error keeps the server's message (with its SQLSTATE or error number).
pub(crate) fn map_err(e: sqlx::Error) -> Error {
    match e {
        sqlx::Error::Database(d) => {
            let code = d.code().map(|c| c.into_owned()).unwrap_or_default();
            // 28000/28P01: PostgreSQL authentication; 1045: MySQL access denied.
            if matches!(code.as_str(), "28000" | "28P01" | "1045" | "1044") {
                Error::Auth(d.message().to_string())
            } else if code.is_empty() {
                Error::Other(d.message().to_string())
            } else {
                Error::Other(format!("{} ({code})", d.message()))
            }
        }
        sqlx::Error::Io(e) => Error::Network(e.to_string()),
        sqlx::Error::Tls(e) => Error::Network(format!("TLS: {e}")),
        sqlx::Error::PoolTimedOut => Error::Network("timed out connecting".into()),
        e => Error::Other(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols() -> Vec<ResultColumn> {
        vec![ResultColumn {
            name: "a".into(),
            type_name: "INT".into(),
        }]
    }

    #[test]
    fn collector_cuts_rows_and_keeps_statements_apart() {
        let mut c = Collector::new(QueryLimits {
            max_rows: 2,
            max_bytes: 1 << 20,
        });
        assert!(c.row(cols, || vec![Value::Text("1".into())]));
        assert!(c.row(cols, || vec![Value::Text("2".into())]));
        assert!(!c.row(cols, || vec![Value::Text("3".into())]));
        assert!(c.truncated());
        c.done(0);
        c.done(5);
        let out = c.finish(Duration::from_millis(7));
        assert_eq!(out.results.len(), 2);
        assert_eq!(out.results[0].rows.len(), 2);
        assert!(out.results[0].truncated);
        assert_eq!(out.results[1].rows_affected, 5);
        assert!(out.results[1].columns.is_empty());
        assert_eq!(out.elapsed_ms, 7);
    }

    #[test]
    fn values_serialize_compactly() {
        let v = vec![
            Value::Null,
            Value::Text("x".into()),
            Value::from_bytes(&[0xff, 0x00]),
        ];
        assert_eq!(
            serde_json::to_string(&v).unwrap(),
            r#"[null,"x",{"hex":"ff00"}]"#
        );
    }

    #[test]
    fn params_debug_hides_the_password() {
        let p = DbParams {
            kind: DbKind::Mysql,
            host: "h".into(),
            port: 3306,
            user: "u".into(),
            password: Zeroizing::new("hunter2".into()),
            database: None,
            tls: DbTls::Prefer,
            verify_host: true,
            connect_timeout: Duration::from_secs(5),
        };
        assert!(!format!("{p:?}").contains("hunter2"));
    }
}
