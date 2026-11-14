//! PostgreSQL.

use crate::{
    map_err, sql::EditStatement, Collector, ColumnInfo, DbParams, DbTls, QueryLimits, QueryOutput,
    Result, ResultColumn, TableInfo, Value,
};
use aht_core::Error;
use futures::future::BoxFuture;
use futures::TryStreamExt;
use sqlx::postgres::{PgConnectOptions, PgConnection, PgRow, PgSslMode};
use sqlx::{Column, Connection, Either, Executor, Row, TypeInfo, ValueRef};
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

pub(crate) fn values(row: &PgRow) -> Vec<Value> {
    (0..row.len()).map(|i| value(row, i)).collect()
}

pub(crate) fn texts(row: &PgRow) -> Vec<Option<String>> {
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

    pub(crate) async fn open(&self) -> Result<PgConnection> {
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
                let id = (&mut conn)
                    .fetch_all(sqlx::raw_sql("SELECT pg_backend_pid()::text"))
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
            let mut q = sqlx::query::<sqlx::Postgres>(sql);
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
        let rows = self.side_rows("SELECT version()", &[]).await?;
        Ok(rows.first().map(|r| text_at(r, 0)).unwrap_or_default())
    }

    pub(crate) fn current_database(&self) -> BoxFuture<'_, Result<Option<String>>> {
        Box::pin(async move {
            let mut slot = self.main.lock().await;
            let conn = self.ensure_main(&mut slot).await?;
            let rows = (&mut *conn)
                .fetch_all(sqlx::raw_sql("SELECT current_database()"))
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
                "SELECT c.relname::text, CASE WHEN c.relkind IN ('v', 'm') THEN 'YES' ELSE 'NO' END, \
                 CASE WHEN c.relkind IN ('r', 'p', 'm') THEN c.reltuples::bigint::text END, \
                 CASE WHEN c.relkind IN ('r', 'p', 'm') THEN pg_total_relation_size(c.oid)::text END, \
                 obj_description(c.oid, 'pg_class'), c.relkind::text \
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
                // -1: never analysed.
                rows: crate::count(r.get(2).and_then(|v| v.as_ref())),
                size: crate::count(r.get(3).and_then(|v| v.as_ref())),
                engine: None,
                comment: crate::non_empty(r.get(4).and_then(|v| v.as_ref())),
                materialized: text_at(r, 5) == "m",
            })
            .collect())
    }

    pub(crate) async fn server_info(&self, what: crate::ServerInfo) -> Result<crate::ResultSet> {
        use crate::ServerInfo::*;
        let (columns, sql): (&[&str], &str) = match what {
            Processes => (
                &[
                    "pid", "user", "client", "db", "state", "seconds", "wait", "query",
                ],
                "SELECT pid::text, usename::text, client_addr::text, datname::text, state, \
                 floor(extract(epoch FROM now() - query_start))::bigint::text, \
                 wait_event_type || ':' || wait_event, left(query, 2000) \
                 FROM pg_stat_activity WHERE backend_type = 'client backend' \
                 ORDER BY query_start NULLS LAST, pid",
            ),
            Variables => (
                &["name", "value", "unit", "description"],
                "SELECT name, setting, unit, short_desc FROM pg_settings ORDER BY name",
            ),
            Status => (
                &["name", "value"],
                "SELECT j.key, j.value FROM pg_stat_database d, \
                 json_each_text(row_to_json(d)) j WHERE d.datname = current_database()",
            ),
            Users => (
                &[
                    "role",
                    "login",
                    "superuser",
                    "create role",
                    "create db",
                    "replication",
                    "valid until",
                ],
                "SELECT rolname::text, rolcanlogin::text, rolsuper::text, rolcreaterole::text, \
                 rolcreatedb::text, rolreplication::text, rolvaliduntil::text \
                 FROM pg_roles WHERE rolname NOT LIKE 'pg\\_%' ORDER BY 1",
            ),
        };
        let rows = self.side_rows(sql, &[]).await?;
        Ok(crate::text_result(columns, rows))
    }

    /// Run statements without results on the side connection, as one simple query (one
    /// implicit transaction).
    pub(crate) fn side_exec<'a>(&'a self, sql: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let mut slot = self.side.lock().await;
            if slot.is_none() {
                *slot = Some(self.open().await?);
            }
            let conn = slot.as_mut().expect("connected above");
            match (&mut *conn).execute(sqlx::raw_sql(sql)).await {
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

    pub(crate) async fn role_grants(&self, role: &str) -> Result<crate::ResultSet> {
        let rows = self
            .side_rows(
                "SELECT table_schema::text, table_name::text, \
                 string_agg(privilege_type::text, ', ' ORDER BY privilege_type) \
                 FROM information_schema.role_table_grants WHERE grantee = $1 \
                 GROUP BY 1, 2 ORDER BY 1, 2",
                &[role],
            )
            .await?;
        Ok(crate::text_result(&["schema", "table", "privileges"], rows))
    }

    pub(crate) async fn kill(&self, pid: u64, connection: bool) -> Result<()> {
        let pid = i32::try_from(pid)
            .map_err(|_| Error::Invalid(format!("no process {pid}")))?
            .to_string();
        let sql = if connection {
            "SELECT pg_terminate_backend($1::int)::text"
        } else {
            "SELECT pg_cancel_backend($1::int)::text"
        };
        let rows = self.side_rows(sql, &[&pid]).await?;
        if rows.first().map(|r| text_at(r, 0)).as_deref() != Some("true") {
            return Err(Error::NotFound(format!("process {pid}")));
        }
        Ok(())
    }

    pub(crate) async fn structure(
        &self,
        schema: &str,
        table: &str,
    ) -> Result<crate::TableStructure> {
        let at = |r: &Vec<Option<String>>, i: usize| r.get(i).cloned().flatten();
        let columns = self
            .side_rows(
                "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod), \
                 CASE WHEN a.attnotnull THEN 'NO' ELSE 'YES' END, \
                 CASE WHEN a.attgenerated = '' THEN pg_get_expr(d.adbin, d.adrelid) END, \
                 CASE WHEN EXISTS (SELECT 1 FROM pg_index i WHERE i.indrelid = c.oid \
                   AND i.indisprimary AND a.attnum = ANY (i.indkey)) THEN 'YES' ELSE 'NO' END, \
                 CASE a.attidentity WHEN 'a' THEN 'GENERATED ALWAYS AS IDENTITY' \
                   WHEN 'd' THEN 'GENERATED BY DEFAULT AS IDENTITY' \
                   ELSE CASE WHEN a.attgenerated <> '' \
                     THEN 'GENERATED ALWAYS AS (' || pg_get_expr(d.adbin, d.adrelid) || ')' END END, \
                 col_description(c.oid, a.attnum) \
                 FROM pg_attribute a \
                 JOIN pg_class c ON c.oid = a.attrelid \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
                 WHERE n.nspname = $1 AND c.relname = $2 AND a.attnum > 0 AND NOT a.attisdropped \
                 ORDER BY a.attnum",
                &[schema, table],
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
                collation: None,
            })
            .collect();
        let indexes = self
            .side_rows(
                "SELECT ic.relname::text, CASE WHEN ix.indisunique THEN 'YES' ELSE 'NO' END, \
                 CASE WHEN ix.indisprimary THEN 'YES' ELSE 'NO' END, am.amname::text, \
                 array_to_string(ARRAY(SELECT pg_get_indexdef(ix.indexrelid, k, true) \
                   FROM generate_series(1, ix.indnkeyatts) k ORDER BY k), E'\\x1f') \
                 FROM pg_index ix \
                 JOIN pg_class ic ON ic.oid = ix.indexrelid \
                 JOIN pg_am am ON am.oid = ic.relam \
                 JOIN pg_class c ON c.oid = ix.indrelid \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = $1 AND c.relname = $2 \
                 ORDER BY ix.indisprimary DESC, 1",
                &[schema, table],
            )
            .await?
            .iter()
            .map(|r| crate::IndexInfo {
                name: text_at(r, 0),
                unique: text_at(r, 1) == "YES",
                primary: text_at(r, 2) == "YES",
                kind: text_at(r, 3),
                columns: crate::split_list(r.get(4).and_then(|v| v.as_ref())),
            })
            .collect();
        let rule = |c: String| {
            match c.as_str() {
                "c" => "CASCADE",
                "n" => "SET NULL",
                "d" => "SET DEFAULT",
                "r" => "RESTRICT",
                _ => "NO ACTION",
            }
            .to_string()
        };
        let foreign_keys = self
            .side_rows(
                "SELECT k.conname::text, \
                 array_to_string(ARRAY(SELECT a.attname::text FROM unnest(k.conkey) WITH ORDINALITY u(n, o) \
                   JOIN pg_attribute a ON a.attrelid = k.conrelid AND a.attnum = u.n ORDER BY u.o), E'\\x1f'), \
                 fn.nspname::text, fc.relname::text, \
                 array_to_string(ARRAY(SELECT a.attname::text FROM unnest(k.confkey) WITH ORDINALITY u(n, o) \
                   JOIN pg_attribute a ON a.attrelid = k.confrelid AND a.attnum = u.n ORDER BY u.o), E'\\x1f'), \
                 k.confupdtype::text, k.confdeltype::text \
                 FROM pg_constraint k \
                 JOIN pg_class c ON c.oid = k.conrelid JOIN pg_namespace n ON n.oid = c.relnamespace \
                 JOIN pg_class fc ON fc.oid = k.confrelid JOIN pg_namespace fn ON fn.oid = fc.relnamespace \
                 WHERE n.nspname = $1 AND c.relname = $2 AND k.contype = 'f' ORDER BY 1",
                &[schema, table],
            )
            .await?
            .iter()
            .map(|r| crate::ForeignKeyInfo {
                name: text_at(r, 0),
                columns: crate::split_list(r.get(1).and_then(|v| v.as_ref())),
                ref_namespace: text_at(r, 2),
                ref_table: text_at(r, 3),
                ref_columns: crate::split_list(r.get(4).and_then(|v| v.as_ref())),
                on_update: rule(text_at(r, 5)),
                on_delete: rule(text_at(r, 6)),
            })
            .collect();
        Ok(crate::TableStructure {
            columns,
            indexes,
            foreign_keys,
        })
    }

    /// PostgreSQL has no `SHOW CREATE TABLE`: views come from `pg_get_viewdef`, tables are
    /// rebuilt from their columns, constraints and the indexes no constraint owns.
    pub(crate) async fn ddl(&self, schema: &str, table: &str) -> Result<String> {
        Ok(self.ddl_parts(schema, table, false).await?.0)
    }

    /// The `CREATE` statement and, with `split_fks`, the foreign keys apart as
    /// `ALTER TABLE ... ADD CONSTRAINT` (a dump adds them once every table holds its rows).
    pub(crate) async fn ddl_parts(
        &self,
        schema: &str,
        table: &str,
        split_fks: bool,
    ) -> Result<(String, Vec<String>)> {
        let name = crate::sql::qualified(crate::DbKind::Postgres, schema, table);
        let kind = self
            .side_rows(
                "SELECT c.relkind::text, \
                 CASE WHEN c.relkind IN ('v', 'm') THEN pg_get_viewdef(c.oid, true) END FROM pg_class c \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = $1 AND c.relname = $2",
                &[schema, table],
            )
            .await?;
        let Some(row) = kind.first() else {
            return Err(Error::NotFound(format!("table {schema}.{table}")));
        };
        match text_at(row, 0).as_str() {
            "v" => {
                return Ok((
                    format!("CREATE VIEW {name} AS\n{}", text_at(row, 1).trim_end()),
                    Vec::new(),
                ))
            }
            "m" => {
                return Ok((
                    format!(
                        "CREATE MATERIALIZED VIEW {name} AS\n{}",
                        text_at(row, 1).trim_end()
                    ),
                    Vec::new(),
                ))
            }
            _ => {}
        }
        let columns = self.columns(schema, table).await?;
        // Identity ('a' always, 'd' by default) and generated ('s' stored, 'v' virtual)
        // columns: their default is not a DEFAULT clause.
        let kinds: std::collections::HashMap<String, (String, String)> = self
            .side_rows(
                "SELECT a.attname::text, a.attidentity::text, a.attgenerated::text \
                 FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = $1 AND c.relname = $2 AND a.attnum > 0 AND NOT a.attisdropped",
                &[schema, table],
            )
            .await?
            .iter()
            .map(|r| (text_at(r, 0), (text_at(r, 1), text_at(r, 2))))
            .collect();
        let constraints = self
            .side_rows(
                "SELECT k.conname::text, pg_get_constraintdef(k.oid, true), k.contype::text \
                 FROM pg_constraint k \
                 JOIN pg_class c ON c.oid = k.conrelid \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = $1 AND c.relname = $2 AND k.contype <> 'n' \
                 ORDER BY CASE k.contype WHEN 'p' THEN 0 WHEN 'u' THEN 1 WHEN 'f' THEN 2 ELSE 3 END, 1",
                &[schema, table],
            )
            .await?;
        let indexes = self
            .side_rows(
                "SELECT pg_get_indexdef(i.indexrelid) FROM pg_index i \
                 JOIN pg_class c ON c.oid = i.indrelid \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = $1 AND c.relname = $2 \
                 AND NOT EXISTS (SELECT 1 FROM pg_constraint k WHERE k.conindid = i.indexrelid) \
                 ORDER BY 1",
                &[schema, table],
            )
            .await?;
        let q = |n: &str| crate::sql::quote_ident(crate::DbKind::Postgres, n);
        let mut lines: Vec<String> = columns
            .iter()
            .map(|c| {
                let mut l = format!("    {} {}", q(&c.name), c.data_type);
                let (identity, generated) = kinds.get(&c.name).cloned().unwrap_or_default();
                match (identity.as_str(), generated.as_str(), &c.default) {
                    ("a", _, _) => l.push_str(" GENERATED ALWAYS AS IDENTITY"),
                    ("d", _, _) => l.push_str(" GENERATED BY DEFAULT AS IDENTITY"),
                    (_, "s", Some(e)) => l.push_str(&format!(" GENERATED ALWAYS AS ({e}) STORED")),
                    (_, "v", Some(e)) => l.push_str(&format!(" GENERATED ALWAYS AS ({e}) VIRTUAL")),
                    _ => {}
                }
                // An identity column is NOT NULL by definition.
                if !c.nullable && identity.is_empty() {
                    l.push_str(" NOT NULL");
                }
                if let (Some(d), "", "") = (&c.default, identity.as_str(), generated.as_str()) {
                    l.push_str(&format!(" DEFAULT {d}"));
                }
                l
            })
            .collect();
        let mut fks = Vec::new();
        for k in &constraints {
            if split_fks && text_at(k, 2) == "f" {
                fks.push(format!(
                    "ALTER TABLE {name} ADD CONSTRAINT {} {};",
                    q(&text_at(k, 0)),
                    text_at(k, 1)
                ));
                continue;
            }
            lines.push(format!(
                "    CONSTRAINT {} {}",
                q(&text_at(k, 0)),
                text_at(k, 1)
            ));
        }
        let mut out = format!("CREATE TABLE {name} (\n{}\n);", lines.join(",\n"));
        for i in &indexes {
            out.push_str(&format!("\n{};", text_at(i, 0)));
        }
        Ok((out, fks))
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
                            // After a cut the statement ends with "canceling statement": not an error in itself,
                            // but the rest of the run did not happen.
                            if stopped && out.truncated() && !lost {
                                out.stopped_after_cut(crate::DbKind::Postgres, sql);
                            } else {
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
            let sql = format!("SELECT pg_cancel_backend({id})");
            match (&mut *conn).execute(sqlx::raw_sql(&sql)).await {
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
    /// delete meant for one row that would change several rolls everything back; so does one that finds no row (changed or deleted meanwhile).
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
            if let Err(e) = (&mut *conn).execute(sqlx::raw_sql("BEGIN")).await {
                if broken(&e) {
                    *slot = None;
                }
                return Err(map_err(e));
            }
            let mut total = 0u64;
            let mut failure: Option<(Error, bool)> = None;
            for st in statements {
                let mut q = sqlx::query::<sqlx::Postgres>(&st.sql);
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
                        if st.single_row && n == 0 {
                            failure = Some((
                                Error::Invalid(
                                    "a row was changed or deleted meanwhile; nothing was saved"
                                        .into(),
                                ),
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
