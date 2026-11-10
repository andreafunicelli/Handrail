//! MySQL and MariaDB.

use crate::{
    map_err, sql::EditStatement, Collector, ColumnInfo, DbParams, DbTls, QueryLimits, QueryOutput,
    Result, ResultColumn, TableInfo, Value,
};
use aht_core::Error;
use futures::future::BoxFuture;
use futures::TryStreamExt;
use sqlx::mysql::{MySqlConnectOptions, MySqlConnection, MySqlRow, MySqlSslMode};
use sqlx::{Column, Connection, Either, Executor, Row, TypeInfo, ValueRef};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

pub(crate) struct Client {
    opts: MySqlConnectOptions,
    timeout: Duration,
    main: Mutex<Option<MySqlConnection>>,
    side: Mutex<Option<MySqlConnection>>,
    /// `CONNECTION_ID()` of the main connection, for `KILL QUERY`; 0 while unknown.
    main_id: AtomicU64,
}

fn options(p: &DbParams) -> MySqlConnectOptions {
    let ssl = match p.tls {
        DbTls::Disable => MySqlSslMode::Disabled,
        DbTls::Prefer => MySqlSslMode::Preferred,
        DbTls::Require => MySqlSslMode::Required,
        DbTls::Verify if p.verify_host => MySqlSslMode::VerifyIdentity,
        DbTls::Verify => MySqlSslMode::VerifyCa,
    };
    let mut o = MySqlConnectOptions::new()
        .host(&p.host)
        .port(p.port)
        .username(&p.user)
        .password(&p.password)
        .ssl_mode(ssl)
        // A console must not change the session: sqlx otherwise sets PIPES_AS_CONCAT,
        // NO_ENGINE_SUBSTITUTION and a UTC time zone.
        .pipes_as_concat(false)
        .no_engine_substitution(false)
        .timezone(None::<String>)
        .statement_cache_capacity(0);
    if let Some(db) = p.database.as_deref().filter(|d| !d.is_empty()) {
        o = o.database(db);
    }
    o
}

/// Whether the connection is unusable after this error.
fn broken(e: &sqlx::Error) -> bool {
    matches!(
        e,
        sqlx::Error::Io(_) | sqlx::Error::Protocol(_) | sqlx::Error::Tls(_)
    )
}

fn columns(row: &MySqlRow) -> Vec<ResultColumn> {
    row.columns()
        .iter()
        .map(|c| ResultColumn {
            name: c.name().to_string(),
            type_name: c.type_info().name().to_string(),
        })
        .collect()
}

fn value(row: &MySqlRow, i: usize) -> Value {
    match row.try_get_raw(i) {
        Ok(v) if v.is_null() => Value::Null,
        // The text protocol sends every value as text; decoding raw bytes skips the type check.
        Ok(_) => row
            .try_get_unchecked::<&[u8], _>(i)
            .map_or(Value::Null, Value::from_bytes),
        Err(_) => Value::Null,
    }
}

pub(crate) fn values(row: &MySqlRow) -> Vec<Value> {
    (0..row.len()).map(|i| value(row, i)).collect()
}

pub(crate) fn texts(row: &MySqlRow) -> Vec<Option<String>> {
    (0..row.len())
        .map(|i| match value(row, i) {
            Value::Null => None,
            Value::Text(s) => Some(s),
            Value::Bytes { hex } => Some(hex),
        })
        .collect()
}

fn text_at(row: &[Option<String>], i: usize) -> String {
    row.get(i).cloned().flatten().unwrap_or_default()
}

impl Client {
    pub(crate) async fn connect(p: &DbParams) -> Result<Client> {
        let client = Client {
            opts: options(p),
            timeout: p.connect_timeout,
            main: Mutex::new(None),
            side: Mutex::new(None),
            main_id: AtomicU64::new(0),
        };
        let mut main = client.main.lock().await;
        client.ensure_main(&mut main).await?;
        drop(main);
        Ok(client)
    }

    pub(crate) async fn open(&self) -> Result<MySqlConnection> {
        tokio::time::timeout(self.timeout, MySqlConnection::connect_with(&self.opts))
            .await
            .map_err(|_| Error::Network("timed out connecting to the database".into()))?
            .map_err(map_err)
    }

    /// The main connection, reconnecting if the last one broke.
    fn ensure_main<'a>(
        &'a self,
        slot: &'a mut Option<MySqlConnection>,
    ) -> BoxFuture<'a, Result<&'a mut MySqlConnection>> {
        Box::pin(async move {
            if slot.is_none() {
                let mut conn = self.open().await?;
                let id = (&mut conn)
                    .fetch_all(sqlx::raw_sql("SELECT CAST(CONNECTION_ID() AS CHAR)"))
                    .await
                    .map_err(map_err)?
                    .first()
                    .and_then(|r| texts(r).into_iter().next().flatten())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                self.main_id.store(id, Ordering::Relaxed);
                *slot = Some(conn);
            }
            Ok(slot.as_mut().expect("connected above"))
        })
    }

    /// Rows of a schema query on the side connection, every value as text.
    pub(crate) fn side_rows<'a>(
        &'a self,
        sql: &'a str,
        binds: &'a [&'a str],
    ) -> BoxFuture<'a, Result<Vec<Vec<Option<String>>>>> {
        Box::pin(async move {
            let mut slot = self.side.lock().await;
            if slot.is_none() {
                *slot = Some(self.open().await?);
            }
            let conn = slot.as_mut().expect("connected above");
            let mut q = sqlx::query::<sqlx::MySql>(sql);
            for b in binds {
                q = q.bind(*b);
            }
            match (&mut *conn).fetch_all(q).await {
                Ok(rows) => Ok(rows.iter().map(texts).collect()),
                Err(e) => {
                    if broken(&e) {
                        *slot = None;
                    }
                    Err(map_err(e))
                }
            }
        })
    }

    pub(crate) async fn version(&self) -> Result<String> {
        let rows = self.side_rows("SELECT VERSION()", &[]).await?;
        Ok(rows.first().map(|r| text_at(r, 0)).unwrap_or_default())
    }

    pub(crate) fn current_database(&self) -> BoxFuture<'_, Result<Option<String>>> {
        Box::pin(async move {
            let mut slot = self.main.lock().await;
            let conn = self.ensure_main(&mut slot).await?;
            let rows = (&mut *conn)
                .fetch_all(sqlx::raw_sql("SELECT DATABASE()"))
                .await
                .map_err(map_err)?;
            Ok(rows
                .first()
                .and_then(|r| texts(r).into_iter().next().flatten()))
        })
    }

    pub(crate) async fn databases(&self) -> Result<Vec<String>> {
        let rows = self
            .side_rows(
                "SELECT SCHEMA_NAME FROM information_schema.SCHEMATA ORDER BY SCHEMA_NAME",
                &[],
            )
            .await?;
        Ok(rows.iter().map(|r| text_at(r, 0)).collect())
    }

    pub(crate) async fn tables(&self, db: &str) -> Result<Vec<TableInfo>> {
        let rows = self
            .side_rows(
                "SELECT TABLE_NAME, IF(TABLE_TYPE = 'BASE TABLE', 'NO', 'YES'), \
                 CAST(TABLE_ROWS AS CHAR), CAST(DATA_LENGTH + INDEX_LENGTH AS CHAR), ENGINE, \
                 TABLE_COMMENT \
                 FROM information_schema.TABLES WHERE TABLE_SCHEMA = ? ORDER BY TABLE_NAME",
                &[db],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| {
                let view = text_at(r, 1) == "YES";
                TableInfo {
                    name: text_at(r, 0),
                    view,
                    rows: if view {
                        None
                    } else {
                        crate::count(r.get(2).and_then(|v| v.as_ref()))
                    },
                    size: if view {
                        None
                    } else {
                        crate::count(r.get(3).and_then(|v| v.as_ref()))
                    },
                    engine: crate::non_empty(r.get(4).and_then(|v| v.as_ref())),
                    // A view's comment is the word VIEW.
                    comment: if view {
                        None
                    } else {
                        crate::non_empty(r.get(5).and_then(|v| v.as_ref()))
                    },
                    materialized: false,
                }
            })
            .collect())
    }

    pub(crate) async fn server_info(&self, what: crate::ServerInfo) -> Result<crate::ResultSet> {
        use crate::ServerInfo::*;
        let (columns, sql): (&[&str], &str) = match what {
            Processes => (
                &[
                    "id", "user", "host", "db", "command", "seconds", "state", "query",
                ],
                "SELECT CAST(ID AS CHAR), USER, HOST, DB, COMMAND, CAST(TIME AS CHAR), STATE, \
                 LEFT(INFO, 2000) FROM information_schema.PROCESSLIST ORDER BY TIME DESC, ID",
            ),
            Variables => (&["name", "value"], "SHOW GLOBAL VARIABLES"),
            Status => (&["name", "value"], "SHOW GLOBAL STATUS"),
            Users => (
                &["user", "host"],
                "SELECT User, Host FROM mysql.user ORDER BY User, Host",
            ),
        };
        let rows = self.side_text_rows(sql).await?;
        Ok(crate::text_result(columns, rows))
    }

    /// Rows of SQL without parameters, over the text protocol (statements such as SHOW and
    /// KILL that are not all preparable).
    pub(crate) fn side_text_rows<'a>(
        &'a self,
        sql: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Vec<Option<String>>>>> {
        Box::pin(async move {
            let mut slot = self.side.lock().await;
            if slot.is_none() {
                *slot = Some(self.open().await?);
            }
            let conn = slot.as_mut().expect("connected above");
            match (&mut *conn).fetch_all(sqlx::raw_sql(sql)).await {
                Ok(rows) => Ok(rows.iter().map(texts).collect()),
                Err(e) => {
                    if broken(&e) {
                        *slot = None;
                    }
                    Err(map_err(e))
                }
            }
        })
    }

    pub(crate) async fn sql_mode(&self) -> Result<String> {
        let rows = self.side_text_rows("SELECT @@SESSION.sql_mode").await?;
        Ok(rows.first().map(|r| text_at(r, 0)).unwrap_or_default())
    }

    /// Run a statement without results on the side connection (text protocol).
    pub(crate) async fn side_exec(&self, sql: &str) -> Result<()> {
        self.side_text_rows(sql).await.map(|_| ())
    }

    pub(crate) async fn side_text(&self, sql: &str, columns: &[&str]) -> Result<crate::ResultSet> {
        let rows = self.side_text_rows(sql).await?;
        Ok(crate::text_result(columns, rows))
    }

    pub(crate) async fn kill(&self, pid: u64, connection: bool) -> Result<()> {
        // A number, so nothing but the id reaches the statement.
        let sql = format!("KILL {}{pid}", if connection { "" } else { "QUERY " });
        self.side_text_rows(&sql).await.map(|_| ())
    }

    pub(crate) async fn structure(&self, db: &str, table: &str) -> Result<crate::TableStructure> {
        let at = |r: &Vec<Option<String>>, i: usize| r.get(i).cloned().flatten();
        let columns = self
            .side_rows(
                "SELECT COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_DEFAULT, \
                 IF(COLUMN_KEY = 'PRI', 'YES', 'NO'), EXTRA, COLUMN_COMMENT, COLLATION_NAME \
                 FROM information_schema.COLUMNS \
                 WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? ORDER BY ORDINAL_POSITION",
                &[db, table],
            )
            .await?
            .iter()
            .map(|r| crate::ColumnDetail {
                name: text_at(r, 0),
                data_type: text_at(r, 1),
                nullable: text_at(r, 2) == "YES",
                default: at(r, 3),
                primary_key: text_at(r, 4) == "YES",
                extra: crate::non_empty(at(r, 5).as_ref()),
                comment: crate::non_empty(at(r, 6).as_ref()),
                collation: crate::non_empty(at(r, 7).as_ref()),
            })
            .collect();
        let rows = self
            .side_rows(
                "SELECT INDEX_NAME, CAST(NON_UNIQUE AS CHAR), \
                 COALESCE(COLUMN_NAME, '(expression)'), INDEX_TYPE \
                 FROM information_schema.STATISTICS \
                 WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? ORDER BY INDEX_NAME = 'PRIMARY' DESC, \
                 INDEX_NAME, SEQ_IN_INDEX",
                &[db, table],
            )
            .await?;
        let mut indexes: Vec<crate::IndexInfo> = Vec::new();
        for r in &rows {
            let name = text_at(r, 0);
            let column = text_at(r, 2);
            match indexes.last_mut() {
                Some(i) if i.name == name => i.columns.push(column),
                _ => indexes.push(crate::IndexInfo {
                    primary: name == "PRIMARY",
                    unique: text_at(r, 1) == "0",
                    kind: text_at(r, 3),
                    columns: vec![column],
                    name,
                }),
            }
        }
        let rows = self
            .side_rows(
                "SELECT k.CONSTRAINT_NAME, k.COLUMN_NAME, k.REFERENCED_TABLE_SCHEMA, \
                 k.REFERENCED_TABLE_NAME, k.REFERENCED_COLUMN_NAME, r.UPDATE_RULE, r.DELETE_RULE \
                 FROM information_schema.KEY_COLUMN_USAGE k \
                 JOIN information_schema.REFERENTIAL_CONSTRAINTS r \
                   ON r.CONSTRAINT_SCHEMA = k.CONSTRAINT_SCHEMA AND r.CONSTRAINT_NAME = k.CONSTRAINT_NAME \
                   AND r.TABLE_NAME = k.TABLE_NAME \
                 WHERE k.TABLE_SCHEMA = ? AND k.TABLE_NAME = ? AND k.REFERENCED_TABLE_NAME IS NOT NULL \
                 ORDER BY k.CONSTRAINT_NAME, k.ORDINAL_POSITION",
                &[db, table],
            )
            .await?;
        let mut foreign_keys: Vec<crate::ForeignKeyInfo> = Vec::new();
        for r in &rows {
            let name = text_at(r, 0);
            match foreign_keys.last_mut() {
                Some(f) if f.name == name => {
                    f.columns.push(text_at(r, 1));
                    f.ref_columns.push(text_at(r, 4));
                }
                _ => foreign_keys.push(crate::ForeignKeyInfo {
                    name,
                    columns: vec![text_at(r, 1)],
                    ref_namespace: text_at(r, 2),
                    ref_table: text_at(r, 3),
                    ref_columns: vec![text_at(r, 4)],
                    on_update: text_at(r, 5),
                    on_delete: text_at(r, 6),
                }),
            }
        }
        Ok(crate::TableStructure {
            columns,
            indexes,
            foreign_keys,
        })
    }

    pub(crate) async fn ddl(&self, db: &str, table: &str) -> Result<String> {
        let sql = format!(
            "SHOW CREATE TABLE {}",
            crate::sql::qualified(crate::DbKind::Mysql, db, table)
        );
        let rows = self.side_rows(&sql, &[]).await?;
        // Columns: Table, Create Table (or View, Create View, ...).
        Ok(rows.first().map(|r| text_at(r, 1)).unwrap_or_default())
    }

    pub(crate) async fn columns(&self, db: &str, table: &str) -> Result<Vec<ColumnInfo>> {
        let rows = self
            .side_rows(
                "SELECT COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_DEFAULT, \
                 IF(COLUMN_KEY = 'PRI', 'YES', 'NO') FROM information_schema.COLUMNS \
                 WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? ORDER BY ORDINAL_POSITION",
                &[db, table],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| ColumnInfo {
                name: text_at(r, 0),
                data_type: text_at(r, 1),
                nullable: text_at(r, 2) == "YES",
                default: r.get(3).cloned().flatten(),
                primary_key: text_at(r, 4) == "YES",
            })
            .collect())
    }

    pub(crate) fn query<'a>(
        &'a self,
        sql: &'a str,
        limits: QueryLimits,
    ) -> BoxFuture<'a, Result<QueryOutput>> {
        Box::pin(async move {
            let mut slot = self.main.lock().await;
            let conn = self.ensure_main(&mut slot).await?;
            let started = Instant::now();
            let mut out = Collector::new(limits);
            let mut stopped = false;
            let mut lost = false;
            {
                let mut stream = (&mut *conn).fetch_many(sqlx::raw_sql(sql));
                loop {
                    match stream.try_next().await {
                        Ok(Some(Either::Left(done))) => out.done(done.rows_affected()),
                        Ok(Some(Either::Right(row))) => {
                            let kept = out.row(|| columns(&row), || values(&row));
                            if !kept && !stopped {
                                // Enough rows: stop the statement rather than read the rest.
                                stopped = true;
                                if let Err(e) = self.cancel().await {
                                    tracing::debug!("stopping a cut statement: {e}");
                                }
                            }
                        }
                        Ok(None) => break,
                        Err(e) => {
                            lost = broken(&e);
                            // After a cut the statement ends with "interrupted": not an error.
                            if !(stopped && out.truncated()) || lost {
                                out.fail(map_err(e).to_string());
                            }
                            break;
                        }
                    }
                }
            }
            if lost {
                *slot = None;
                self.main_id.store(0, Ordering::Relaxed);
            }
            Ok(out.finish(started.elapsed()))
        })
    }

    pub(crate) fn cancel(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let id = self.main_id.load(Ordering::Relaxed);
            if id == 0 {
                return Ok(());
            }
            let mut slot = self.side.lock().await;
            if slot.is_none() {
                *slot = Some(self.open().await?);
            }
            let conn = slot.as_mut().expect("connected above");
            match (&mut *conn)
                .execute(sqlx::raw_sql(&format!("KILL QUERY {id}")))
                .await
            {
                Ok(_) => Ok(()),
                Err(e) => {
                    if broken(&e) {
                        *slot = None;
                    }
                    Err(map_err(e))
                }
            }
        })
    }

    /// Run `statements` in one transaction on the side connection: all or nothing. An update or
    /// delete meant for one row that would change several rolls everything back.
    pub(crate) fn apply_edits<'a>(
        &'a self,
        statements: &'a [EditStatement],
    ) -> BoxFuture<'a, Result<u64>> {
        Box::pin(async move {
            let mut slot = self.side.lock().await;
            if slot.is_none() {
                *slot = Some(self.open().await?);
            }
            let conn = slot.as_mut().expect("connected above");
            if let Err(e) = (&mut *conn)
                .execute(sqlx::raw_sql("START TRANSACTION"))
                .await
            {
                if broken(&e) {
                    *slot = None;
                }
                return Err(map_err(e));
            }
            let mut total = 0u64;
            let mut failure: Option<(Error, bool)> = None;
            for st in statements {
                let mut q = sqlx::query::<sqlx::MySql>(&st.sql);
                for p in &st.params {
                    q = q.bind(p.as_deref());
                }
                match (&mut *conn).execute(q).await {
                    Ok(done) => {
                        let n = done.rows_affected();
                        if st.single_row && n > 1 {
                            failure = Some((
                                Error::Invalid(format!(
                                    "an edit would change {n} rows instead of one; nothing was saved"
                                )),
                                false,
                            ));
                            break;
                        }
                        total += n;
                    }
                    Err(e) => {
                        let lost = broken(&e);
                        failure = Some((map_err(e), lost));
                        break;
                    }
                }
            }
            if let Some((e, lost)) = failure {
                if lost {
                    *slot = None;
                } else if let Err(r) = (&mut *conn).execute(sqlx::raw_sql("ROLLBACK")).await {
                    tracing::warn!("rolling back edits: {r}");
                    *slot = None;
                }
                return Err(e);
            }
            match (&mut *conn).execute(sqlx::raw_sql("COMMIT")).await {
                Ok(_) => Ok(total),
                Err(e) => {
                    if broken(&e) {
                        *slot = None;
                    }
                    Err(map_err(e))
                }
            }
        })
    }

    pub(crate) async fn close(self) {
        for conn in [self.main.into_inner(), self.side.into_inner()]
            .into_iter()
            .flatten()
        {
            let _ = conn.close().await;
        }
    }
}
