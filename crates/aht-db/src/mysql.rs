//! MySQL and MariaDB.

use crate::{
    map_err, Collector, ColumnInfo, DbParams, DbTls, QueryLimits, QueryOutput, Result,
    ResultColumn, TableInfo, Value,
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

fn values(row: &MySqlRow) -> Vec<Value> {
    (0..row.len()).map(|i| value(row, i)).collect()
}

fn texts(row: &MySqlRow) -> Vec<Option<String>> {
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

    async fn open(&self) -> Result<MySqlConnection> {
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
    fn side_rows<'a>(
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
                "SELECT TABLE_NAME, IF(TABLE_TYPE = 'BASE TABLE', 'NO', 'YES') \
                 FROM information_schema.TABLES WHERE TABLE_SCHEMA = ? ORDER BY TABLE_NAME",
                &[db],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| TableInfo {
                name: text_at(r, 0),
                view: text_at(r, 1) == "YES",
            })
            .collect())
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

    pub(crate) async fn close(self) {
        for conn in [self.main.into_inner(), self.side.into_inner()]
            .into_iter()
            .flatten()
        {
            let _ = conn.close().await;
        }
    }
}
