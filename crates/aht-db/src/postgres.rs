//! PostgreSQL.

use crate::{
    map_err, Collector, ColumnInfo, DbParams, DbTls, QueryLimits, QueryOutput, Result,
    ResultColumn, TableInfo, Value,
};
use aht_core::Error;
use futures::future::BoxFuture;
use futures::TryStreamExt;
use sqlx::postgres::{PgConnectOptions, PgConnection, PgRow, PgSslMode};
use sqlx::{Column, Connection, Either, Row, TypeInfo, ValueRef};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

pub(crate) struct Client {
    opts: PgConnectOptions,
    timeout: Duration,
    main: Mutex<Option<PgConnection>>,
    side: Mutex<Option<PgConnection>>,
    /// `pg_backend_pid()` of the main connection, for `pg_cancel_backend`; 0 while unknown.
    main_id: AtomicU64,
}

fn options(p: &DbParams) -> PgConnectOptions {
    let ssl = match p.tls {
        DbTls::Disable => PgSslMode::Disable,
        DbTls::Prefer => PgSslMode::Prefer,
        DbTls::Require => PgSslMode::Require,
        DbTls::Verify if p.verify_host => PgSslMode::VerifyFull,
        DbTls::Verify => PgSslMode::VerifyCa,
    };
    // Not `new()`: the password is ours, so ~/.pgpass is not read.
    let mut o = PgConnectOptions::new_without_pgpass()
        .host(&p.host)
        .port(p.port)
        .username(&p.user)
        .password(&p.password)
        .ssl_mode(ssl)
        .application_name("Handrail")
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

fn columns(row: &PgRow) -> Vec<ResultColumn> {
    row.columns()
        .iter()
        .map(|c| ResultColumn {
            name: c.name().to_string(),
            type_name: c.type_info().name().to_string(),
        })
        .collect()
}

fn value(row: &PgRow, i: usize) -> Value {
    match row.try_get_raw(i) {
        Ok(v) if v.is_null() => Value::Null,
        // The simple protocol sends every value as text (the schema queries select text only),
        // so the raw bytes are the value as the server prints it.
        Ok(v) => v.as_bytes().map_or(Value::Null, Value::from_bytes),
        Err(_) => Value::Null,
    }
}

fn values(row: &PgRow) -> Vec<Value> {
    (0..row.len()).map(|i| value(row, i)).collect()
}

fn texts(row: &PgRow) -> Vec<Option<String>> {
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

    async fn open(&self) -> Result<PgConnection> {
        tokio::time::timeout(self.timeout, PgConnection::connect_with(&self.opts))
            .await
            .map_err(|_| Error::Network("timed out connecting to the database".into()))?
            .map_err(map_err)
    }

    /// The main connection, reconnecting if the last one broke.
    fn ensure_main<'a>(
        &'a self,
        slot: &'a mut Option<PgConnection>,
    ) -> BoxFuture<'a, Result<&'a mut PgConnection>> {
        Box::pin(async move {
            if slot.is_none() {
                let mut conn = self.open().await?;
                let id = sqlx::raw_sql("SELECT pg_backend_pid()::text")
                    .fetch_all(&mut conn)
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
            let mut q = sqlx::query::<sqlx::Postgres>(sql);
            for b in binds {
                q = q.bind(*b);
            }
            match q.fetch_all(&mut *conn).await {
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
        let rows = self.side_rows("SELECT version()", &[]).await?;
        Ok(rows.first().map(|r| text_at(r, 0)).unwrap_or_default())
    }

    pub(crate) fn current_database(&self) -> BoxFuture<'_, Result<Option<String>>> {
        Box::pin(async move {
            let mut slot = self.main.lock().await;
            let conn = self.ensure_main(&mut slot).await?;
            let rows = sqlx::raw_sql("SELECT current_database()")
                .fetch_all(&mut *conn)
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
                "SELECT datname::text FROM pg_database \
                 WHERE datallowconn AND NOT datistemplate ORDER BY 1",
                &[],
            )
            .await?;
        Ok(rows.iter().map(|r| text_at(r, 0)).collect())
    }

    pub(crate) async fn schemas(&self) -> Result<Vec<String>> {
        let rows = self
            .side_rows(
                "SELECT nspname::text FROM pg_namespace \
                 WHERE nspname NOT LIKE 'pg\\_%' AND nspname <> 'information_schema' \
                 ORDER BY 1",
                &[],
            )
            .await?;
        Ok(rows.iter().map(|r| text_at(r, 0)).collect())
    }

    pub(crate) async fn tables(&self, schema: &str) -> Result<Vec<TableInfo>> {
        let rows = self
            .side_rows(
                "SELECT c.relname::text, CASE WHEN c.relkind IN ('v', 'm') THEN 'YES' ELSE 'NO' END \
                 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = $1 AND c.relkind IN ('r', 'p', 'v', 'm', 'f') ORDER BY 1",
                &[schema],
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

    pub(crate) async fn columns(&self, schema: &str, table: &str) -> Result<Vec<ColumnInfo>> {
        let rows = self
            .side_rows(
                "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod), \
                 CASE WHEN a.attnotnull THEN 'NO' ELSE 'YES' END, \
                 pg_get_expr(d.adbin, d.adrelid), \
                 CASE WHEN EXISTS (SELECT 1 FROM pg_index i WHERE i.indrelid = c.oid \
                   AND i.indisprimary AND a.attnum = ANY (i.indkey)) THEN 'YES' ELSE 'NO' END \
                 FROM pg_attribute a \
                 JOIN pg_class c ON c.oid = a.attrelid \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
                 WHERE n.nspname = $1 AND c.relname = $2 AND a.attnum > 0 AND NOT a.attisdropped \
                 ORDER BY a.attnum",
                &[schema, table],
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
                let mut stream = sqlx::raw_sql(sql).fetch_many(&mut *conn);
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
                            // After a cut the statement ends with "canceling statement": not an error.
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
            match sqlx::raw_sql(&format!("SELECT pg_cancel_backend({id})"))
                .execute(&mut *conn)
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
