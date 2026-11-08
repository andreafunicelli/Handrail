//! Database client for Handrail's database tabs: MySQL/MariaDB and PostgreSQL.
//!
//! A [`DbClient`] holds two connections: the *main* one runs the user's statements (so `BEGIN`,
//! `USE`, `SET` and temporary tables carry over from one run to the next, as in any SQL
//! console), and a *side* one reads the schema and cancels a running statement.
//!
//! Statements run with the simple (text) protocol: several statements may be sent at once, and
//! every value arrives as text, so any column type can be shown without a decoder per type.

mod mysql;
mod postgres;
pub mod sql;

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
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TableInfo {
    pub name: String,
    pub view: bool,
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
