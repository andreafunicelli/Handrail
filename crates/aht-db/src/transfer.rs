//! Export a namespace to an SQL file, import an SQL or CSV file. Each runs on a connection of
//! its own, so the editor and the schema tree stay usable meanwhile, and reads or writes the
//! file a line or a batch at a time, so its size does not matter (through a tunnel too).

use crate::dump::{self, CsvReader, Inserts, Splitter, Statement};
use crate::sql::{qualified, quote_ident};
use crate::{map_err, DbClient, DbKind, Inner, Result, TableInfo, Value};
use aht_core::Error;
use futures::TryStreamExt;
use serde::{Deserialize, Serialize};
use sqlx::{Connection, Executor};
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};

/// How far a transfer got.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Progress {
    /// The table being read or written.
    pub table: Option<String>,
    pub tables_done: u64,
    pub tables: u64,
    pub rows: u64,
    pub statements: u64,
    /// Bytes written (export) or read (import).
    pub bytes: u64,
    pub errors: u64,
}

/// A statement the server refused.
#[derive(Clone, Debug, Serialize)]
pub struct TransferError {
    /// Where it starts in the file (1-based; 0 for an export).
    pub line: u64,
    /// Its beginning.
    pub statement: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Summary {
    pub progress: Progress,
    /// The first errors (at most [`MAX_ERRORS`]); `progress.errors` counts them all.
    pub errors: Vec<TransferError>,
    pub cancelled: bool,
    /// Nothing was kept: the import ran in a transaction that was rolled back.
    pub rolled_back: bool,
}

pub const MAX_ERRORS: usize = 50;

impl Summary {
    fn error(&mut self, line: u64, statement: &str, message: String) {
        self.progress.errors += 1;
        if self.errors.len() < MAX_ERRORS {
            let mut st: String = statement.chars().take(300).collect();
            if st.len() < statement.len() {
                st.push('…');
            }
            self.errors.push(TransferError {
                line,
                statement: st,
                message,
            });
        }
    }
}

/// Called as a transfer advances (often: throttle what it shows).
pub type Report<'a> = &'a mut (dyn FnMut(&Progress) + Send);

/// What an export holds.
#[derive(Clone, Debug, Deserialize)]
pub struct DumpOptions {
    /// Tables and views to export; empty: all of the namespace.
    #[serde(default)]
    pub tables: Vec<String>,
    pub structure: bool,
    pub data: bool,
    /// Drop each table and view before creating it.
    #[serde(default)]
    pub drop: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ImportOptions {
    /// Stop at the first refused statement. On PostgreSQL the file then runs in one
    /// transaction: all of it or nothing.
    pub stop_on_error: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CsvOptions {
    pub delimiter: char,
    /// The first record names the columns (it is skipped).
    pub header: bool,
    /// A field holding exactly this is NULL (`\N`, an empty string...).
    #[serde(default)]
    pub null: Option<String>,
    /// For each field of a record, the column it goes in (`None`: skipped).
    pub columns: Vec<Option<String>>,
    /// Delete the table's rows first (in the same transaction).
    #[serde(default)]
    pub replace: bool,
}

/// A connection for one transfer.
enum Conn {
    Mysql(sqlx::MySqlConnection),
    Postgres(sqlx::PgConnection),
}

fn broken(e: &sqlx::Error) -> bool {
    matches!(
        e,
        sqlx::Error::Io(_) | sqlx::Error::Protocol(_) | sqlx::Error::Tls(_)
    )
}

impl Conn {
    async fn exec(&mut self, sql: &str) -> std::result::Result<u64, sqlx::Error> {
        match self {
            Conn::Mysql(c) => (&mut *c)
                .execute(sqlx::raw_sql(sql))
                .await
                .map(|r| r.rows_affected()),
            Conn::Postgres(c) => (&mut *c)
                .execute(sqlx::raw_sql(sql))
                .await
                .map(|r| r.rows_affected()),
        }
    }

    async fn run(&mut self, sql: &str) -> Result<()> {
        self.exec(sql).await.map(|_| ()).map_err(map_err)
    }

    async fn texts(&mut self, sql: &str) -> Result<Vec<Vec<Option<String>>>> {
        Ok(match self {
            Conn::Mysql(c) => (&mut *c)
                .fetch_all(sqlx::raw_sql(sql))
                .await
                .map_err(map_err)?
                .iter()
                .map(crate::mysql::texts)
                .collect(),
            Conn::Postgres(c) => (&mut *c)
                .fetch_all(sqlx::raw_sql(sql))
                .await
                .map_err(map_err)?
                .iter()
                .map(crate::postgres::texts)
                .collect(),
        })
    }

    /// Every row of `sql` (text protocol), one at a time; `f` returns false to stop.
    async fn each_row(
        &mut self,
        sql: &str,
        f: &mut (dyn FnMut(Vec<Value>) -> Result<bool> + Send),
    ) -> Result<()> {
        match self {
            Conn::Mysql(c) => {
                let mut rows = (&mut *c).fetch(sqlx::raw_sql(sql));
                while let Some(r) = rows.try_next().await.map_err(map_err)? {
                    if !f(crate::mysql::values(&r))? {
                        break;
                    }
                }
            }
            Conn::Postgres(c) => {
                let mut rows = (&mut *c).fetch(sqlx::raw_sql(sql));
                while let Some(r) = rows.try_next().await.map_err(map_err)? {
                    if !f(crate::postgres::values(&r))? {
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    /// Whether MySQL strings take backslash escapes in this session.
    async fn backslash_escapes(&mut self) -> Result<bool> {
        let rows = self.texts("SELECT @@SESSION.sql_mode").await?;
        let mode = rows
            .first()
            .and_then(|r| r.first().cloned().flatten())
            .unwrap_or_default();
        Ok(!mode.to_ascii_uppercase().contains("NO_BACKSLASH_ESCAPES"))
    }

    async fn close(self) {
        let _ = match self {
            Conn::Mysql(c) => c.close().await,
            Conn::Postgres(c) => c.close().await,
        };
    }
}

/// A column as an export reads it.
struct DumpColumn {
    name: String,
    /// Written as hex (MySQL BLOB, BINARY, BIT, spatial types).
    binary: bool,
    /// Computed by the server: not inserted.
    generated: bool,
    /// PostgreSQL `GENERATED ALWAYS AS IDENTITY`: inserting needs `OVERRIDING SYSTEM VALUE`.
    identity_always: bool,
}

fn at(r: &[Option<String>], i: usize) -> String {
    r.get(i).cloned().flatten().unwrap_or_default()
}

impl DbClient {
    async fn transfer_conn(&self) -> Result<Conn> {
        Ok(match &self.inner {
            Inner::Mysql(c) => Conn::Mysql(c.open().await?),
            Inner::Postgres(c) => Conn::Postgres(c.open().await?),
        })
    }

    async fn dump_columns(&self, ns: &str, table: &str) -> Result<Vec<DumpColumn>> {
        Ok(match &self.inner {
            Inner::Mysql(c) => c
                .side_rows(
                    "SELECT COLUMN_NAME, DATA_TYPE, EXTRA FROM information_schema.COLUMNS \
                     WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? ORDER BY ORDINAL_POSITION",
                    &[ns, table],
                )
                .await?
                .iter()
                .map(|r| {
                    let extra = at(r, 2).to_ascii_uppercase();
                    DumpColumn {
                        name: at(r, 0),
                        binary: dump::mysql_binary_type(&at(r, 1)),
                        // Not DEFAULT_GENERATED: that is a column with an expression default.
                        generated: extra.contains("VIRTUAL")
                            || extra.contains("STORED")
                            || extra.contains("PERSISTENT"),
                        identity_always: false,
                    }
                })
                .collect(),
            Inner::Postgres(c) => c
                .side_rows(
                    "SELECT a.attname::text, a.attgenerated::text, a.attidentity::text \
                     FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid \
                     JOIN pg_namespace n ON n.oid = c.relnamespace \
                     WHERE n.nspname = $1 AND c.relname = $2 AND a.attnum > 0 \
                     AND NOT a.attisdropped ORDER BY a.attnum",
                    &[ns, table],
                )
                .await?
                .iter()
                .map(|r| DumpColumn {
                    name: at(r, 0),
                    binary: false,
                    generated: !at(r, 1).is_empty(),
                    identity_always: at(r, 2) == "a",
                })
                .collect(),
        })
    }

    /// The namespace's views (and materialized views) in an order that creates each after
    /// the ones it reads.
    async fn view_order(&self, ns: &str) -> Result<Vec<String>> {
        Ok(match &self.inner {
            Inner::Mysql(c) => {
                let defs: Vec<(String, String)> = c
                    .side_rows(
                        "SELECT TABLE_NAME, VIEW_DEFINITION FROM information_schema.VIEWS \
                         WHERE TABLE_SCHEMA = ?",
                        &[ns],
                    )
                    .await?
                    .iter()
                    .map(|r| (at(r, 0), at(r, 1)))
                    .collect();
                let names: Vec<String> = defs.iter().map(|(n, _)| n.clone()).collect();
                dump::order_views(
                    defs.iter()
                        .map(|(n, d)| (n.clone(), dump::mysql_view_deps(d, n, &names)))
                        .collect(),
                )
            }
            // What each view reads, from its rewrite rule's dependencies (a view replaced to
            // read a newer one keeps its older oid, so creation order is not enough).
            Inner::Postgres(c) => {
                let mut deps: std::collections::BTreeMap<String, Vec<String>> = c
                    .side_rows(
                        "SELECT c.relname::text FROM pg_class c \
                         JOIN pg_namespace n ON n.oid = c.relnamespace \
                         WHERE n.nspname = $1 AND c.relkind IN ('v', 'm')",
                        &[ns],
                    )
                    .await?
                    .iter()
                    .map(|r| (at(r, 0), Vec::new()))
                    .collect();
                for r in c
                    .side_rows(
                        "SELECT DISTINCT v.relname::text, d.relname::text FROM pg_depend dep \
                         JOIN pg_rewrite rw ON rw.oid = dep.objid \
                         JOIN pg_class v ON v.oid = rw.ev_class \
                         JOIN pg_class d ON d.oid = dep.refobjid \
                         JOIN pg_namespace n ON n.oid = v.relnamespace \
                         WHERE dep.classid = 'pg_rewrite'::regclass \
                           AND dep.refclassid = 'pg_class'::regclass \
                           AND n.nspname = $1 AND d.relnamespace = v.relnamespace \
                           AND v.relkind IN ('v', 'm') AND d.relkind IN ('v', 'm') \
                           AND d.oid <> v.oid",
                        &[ns],
                    )
                    .await?
                {
                    if let Some(list) = deps.get_mut(&at(&r, 0)) {
                        list.push(at(&r, 1));
                    }
                }
                dump::order_views(deps.into_iter().collect())
            }
        })
    }

    /// The `CREATE` of a table or view and, on PostgreSQL, its foreign keys apart.
    async fn dump_ddl(&self, ns: &str, t: &TableInfo) -> Result<(String, Vec<String>)> {
        let (mut create, fks) = match &self.inner {
            Inner::Mysql(c) => {
                let d = c.ddl(ns, &t.name).await?;
                (if t.view { dump::strip_definer(&d) } else { d }, Vec::new())
            }
            Inner::Postgres(c) => c.ddl_parts(ns, &t.name, true).await?,
        };
        create = create.trim_end().to_string();
        if !create.ends_with(';') {
            create.push(';');
        }
        Ok((create, fks))
    }

    /// PostgreSQL sequences of the namespace: (sequence, owning table, owning column,
    /// dependency kind — `a` serial, `i` identity, empty standalone, last value).
    async fn sequences(&self, ns: &str) -> Result<Vec<Vec<Option<String>>>> {
        match &self.inner {
            Inner::Mysql(_) => Ok(Vec::new()),
            Inner::Postgres(c) => {
                c.side_rows(
                    "SELECT s.relname::text, t.relname::text, a.attname::text, d.deptype::text, \
                     ps.last_value::text, format_type(ps.data_type, NULL), \
                     ps.increment_by::text, ps.min_value::text, ps.max_value::text, \
                     ps.start_value::text, ps.cache_size::text, \
                     CASE WHEN ps.cycle THEN 'CYCLE' ELSE 'NO CYCLE' END \
                     FROM pg_class s \
                     JOIN pg_namespace n ON n.oid = s.relnamespace \
                     LEFT JOIN pg_depend d ON d.objid = s.oid \
                       AND d.classid = 'pg_class'::regclass AND d.refclassid = 'pg_class'::regclass \
                       AND d.deptype IN ('a', 'i') \
                     LEFT JOIN pg_class t ON t.oid = d.refobjid \
                     LEFT JOIN pg_attribute a ON a.attrelid = d.refobjid AND a.attnum = d.refobjsubid \
                     LEFT JOIN pg_sequences ps ON ps.schemaname = n.nspname AND ps.sequencename = s.relname \
                     WHERE s.relkind = 'S' AND n.nspname = $1 ORDER BY 1",
                    &[ns],
                )
                .await
            }
        }
    }
}

/// Writes the file and counts what it wrote.
struct Out<'a> {
    w: &'a mut (dyn Write + Send),
    p: Progress,
}

impl Out<'_> {
    fn put(&mut self, s: &str) -> Result<()> {
        self.w.write_all(s.as_bytes())?;
        self.p.bytes += s.len() as u64;
        Ok(())
    }

    fn statement(&mut self, s: &str) -> Result<()> {
        self.put(s)?;
        if !s.ends_with('\n') {
            self.put("\n")?;
        }
        self.p.statements += 1;
        Ok(())
    }
}

/// Write `ns` (its tables, views and, on PostgreSQL, sequences) to `w` as SQL that the
/// import below, the `mysql` client or `psql` runs. Rows are read in one snapshot. Triggers,
/// routines and events are not exported.
pub async fn dump(
    client: &DbClient,
    ns: &str,
    opts: &DumpOptions,
    w: &mut (dyn Write + Send),
    report: Report<'_>,
    cancel: &AtomicBool,
) -> Result<Summary> {
    let kind = client.kind();
    let all = client.tables(ns).await?;
    let picked: Vec<&TableInfo> = all
        .iter()
        .filter(|t| opts.tables.is_empty() || opts.tables.contains(&t.name))
        .collect();
    let tables: Vec<&TableInfo> = picked.iter().copied().filter(|t| !t.view).collect();
    let views: Vec<&TableInfo> = if opts.structure {
        let order = client.view_order(ns).await?;
        order
            .iter()
            .filter_map(|n| picked.iter().copied().find(|t| t.view && &t.name == n))
            .collect()
    } else {
        Vec::new()
    };
    let name = |t: &str| match kind {
        DbKind::Mysql => quote_ident(kind, t),
        DbKind::Postgres => qualified(kind, ns, t),
    };
    let mut out = Out {
        w,
        p: Progress {
            tables: (tables.len() + views.len()) as u64,
            ..Progress::default()
        },
    };
    let mut summary = Summary::default();

    let mut conn = client.transfer_conn().await?;
    let setup: &[&str] = match kind {
        DbKind::Mysql => &[
            "SET NAMES utf8mb4",
            "SET SESSION time_zone = '+00:00'",
            "SET SESSION TRANSACTION ISOLATION LEVEL REPEATABLE READ",
            "START TRANSACTION WITH CONSISTENT SNAPSHOT",
        ],
        DbKind::Postgres => &[
            "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY",
            "SET LOCAL DateStyle = ISO",
            "SET LOCAL IntervalStyle = postgres",
            "SET LOCAL extra_float_digits = 3",
        ],
    };
    for s in setup {
        conn.run(s).await?;
    }

    let version = client.version().await.unwrap_or_default();
    out.put(&format!(
        "-- Handrail SQL dump\n-- Server: {}\n-- {}: {}\n-- Structure: {}, data: {}\n\n",
        version.replace('\n', " "),
        if kind == DbKind::Mysql {
            "Database"
        } else {
            "Schema"
        },
        ns.replace('\n', " "),
        if opts.structure { "yes" } else { "no" },
        if opts.data { "yes" } else { "no" },
    ))?;
    match kind {
        DbKind::Mysql => {
            for s in [
                "SET NAMES utf8mb4;",
                "SET @OLD_TIME_ZONE = @@TIME_ZONE, TIME_ZONE = '+00:00';",
                "SET @OLD_FOREIGN_KEY_CHECKS = @@FOREIGN_KEY_CHECKS, FOREIGN_KEY_CHECKS = 0;",
                // No NO_BACKSLASH_ESCAPES: the strings below escape with backslashes.
                "SET @OLD_SQL_MODE = @@SQL_MODE, SQL_MODE = 'NO_AUTO_VALUE_ON_ZERO';",
            ] {
                out.statement(s)?;
            }
        }
        DbKind::Postgres => {
            for s in [
                "SET client_encoding = 'UTF8';",
                "SET standard_conforming_strings = on;",
                "SET check_function_bodies = false;",
            ] {
                out.statement(s)?;
            }
        }
    }

    let result = dump_body(
        client,
        &mut conn,
        ns,
        opts,
        &tables,
        &views,
        &name,
        &mut out,
        &mut *report,
        cancel,
        &mut summary,
    )
    .await;
    conn.close().await;
    result?;
    if !summary.cancelled && kind == DbKind::Mysql {
        out.put("\n")?;
        for s in [
            "SET SQL_MODE = @OLD_SQL_MODE;",
            "SET FOREIGN_KEY_CHECKS = @OLD_FOREIGN_KEY_CHECKS;",
            "SET TIME_ZONE = @OLD_TIME_ZONE;",
        ] {
            out.statement(s)?;
        }
    }
    out.w.flush()?;
    out.p.table = None;
    report(&out.p);
    summary.progress = out.p;
    Ok(summary)
}

#[allow(clippy::too_many_arguments)]
async fn dump_body(
    client: &DbClient,
    conn: &mut Conn,
    ns: &str,
    opts: &DumpOptions,
    tables: &[&TableInfo],
    views: &[&TableInfo],
    name: &(dyn Fn(&str) -> String + Sync),
    out: &mut Out<'_>,
    report: Report<'_>,
    cancel: &AtomicBool,
    summary: &mut Summary,
) -> Result<()> {
    let kind = client.kind();
    let pg = kind == DbKind::Postgres;
    let cascade = if pg { " CASCADE" } else { "" };
    if opts.structure && opts.drop {
        // Every drop first: on PostgreSQL dropping a table drops its sequences, which the
        // tables below need.
        out.put("\n")?;
        for v in views.iter().rev() {
            let what = if v.materialized {
                "MATERIALIZED VIEW"
            } else {
                "VIEW"
            };
            out.statement(&format!(
                "DROP {what} IF EXISTS {}{cascade};",
                name(&v.name)
            ))?;
        }
        for t in tables {
            out.statement(&format!("DROP TABLE IF EXISTS {}{cascade};", name(&t.name)))?;
        }
    }

    let dumped = |t: &str| tables.iter().any(|x| x.name == t);
    let sequences = if pg {
        client.sequences(ns).await?
    } else {
        Vec::new()
    };
    let lit = |s: &str| dump::text_literal(kind, s, true);
    // Sequences of the tables exported (serial columns), and the standalone ones with the
    // whole schema; identity columns make their own.
    let own_seq = |r: &Vec<Option<String>>| match at(r, 3).as_str() {
        "a" => dumped(&at(r, 1)),
        "" => opts.tables.is_empty(),
        _ => false,
    };
    if opts.structure && pg {
        let mut first = true;
        for r in sequences.iter().filter(|&r| own_seq(r)) {
            if first {
                out.put("\n")?;
                first = false;
            }
            out.statement(&format!(
                "CREATE SEQUENCE IF NOT EXISTS {}{};",
                name(&at(r, 0)),
                sequence_options(r)
            ))?;
        }
    }

    let mut fks = Vec::new();
    for t in tables {
        if cancel.load(Ordering::Relaxed) {
            summary.cancelled = true;
            return Ok(());
        }
        out.p.table = Some(t.name.clone());
        report(&out.p);
        if opts.structure {
            let (create, f) = client.dump_ddl(ns, t).await?;
            out.put(&format!("\n-- Table {}\n\n", t.name.replace('\n', " ")))?;
            out.statement(&create)?;
            fks.extend(f);
        }
        if opts.data && !dump_rows(client, conn, ns, t, name, out, &mut *report, cancel).await? {
            summary.cancelled = true;
            return Ok(());
        }
        out.p.tables_done += 1;
    }

    if opts.structure && pg {
        let owned: Vec<String> = sequences
            .iter()
            .filter(|r| at(r, 3) == "a" && dumped(&at(r, 1)))
            .map(|r| {
                format!(
                    "ALTER SEQUENCE {} OWNED BY {}.{};",
                    name(&at(r, 0)),
                    name(&at(r, 1)),
                    quote_ident(kind, &at(r, 2))
                )
            })
            .collect();
        if !owned.is_empty() || !fks.is_empty() {
            out.put("\n")?;
        }
        for s in owned.iter().chain(&fks) {
            out.statement(s)?;
        }
    }
    if opts.data && pg {
        // Where each sequence was, so new rows do not collide with the ones restored.
        let mut first = true;
        for r in &sequences {
            let Some(last) = r.get(4).cloned().flatten() else {
                continue;
            };
            let target = match at(r, 3).as_str() {
                "i" if dumped(&at(r, 1)) => format!(
                    "pg_get_serial_sequence({}, {})",
                    lit(&name(&at(r, 1))),
                    lit(&at(r, 2))
                ),
                _ if own_seq(r) => lit(&name(&at(r, 0))),
                _ => continue,
            };
            if first {
                out.put("\n")?;
                first = false;
            }
            // A number from the catalog; checked so nothing else reaches the file.
            if last.parse::<i64>().is_ok() {
                out.statement(&format!("SELECT setval({target}, {last}, true);"))?;
            }
        }
    }

    for v in views {
        if cancel.load(Ordering::Relaxed) {
            summary.cancelled = true;
            return Ok(());
        }
        out.p.table = Some(v.name.clone());
        report(&out.p);
        let (create, _) = client.dump_ddl(ns, v).await?;
        out.put(&format!("\n-- View {}\n\n", v.name.replace('\n', " ")))?;
        out.statement(&create)?;
        out.p.tables_done += 1;
    }
    Ok(())
}

/// Write a table's rows as `INSERT`s; false when cancelled.
#[allow(clippy::too_many_arguments)]
async fn dump_rows(
    client: &DbClient,
    conn: &mut Conn,
    ns: &str,
    t: &TableInfo,
    name: &(dyn Fn(&str) -> String + Sync),
    out: &mut Out<'_>,
    report: Report<'_>,
    cancel: &AtomicBool,
) -> Result<bool> {
    let kind = client.kind();
    let cols = client.dump_columns(ns, &t.name).await?;
    let kept: Vec<&DumpColumn> = cols.iter().filter(|c| !c.generated).collect();
    if kept.is_empty() {
        return Ok(true);
    }
    let quoted: Vec<String> = kept.iter().map(|c| quote_ident(kind, &c.name)).collect();
    // ONLY: a PostgreSQL parent table's SELECT would repeat the rows of its children.
    let select = format!(
        "SELECT {} FROM {}{}",
        quoted.join(", "),
        if kind == DbKind::Postgres {
            "ONLY "
        } else {
            ""
        },
        qualified(kind, ns, &t.name)
    );
    let mut ins = Inserts::new(
        &name(&t.name),
        &quoted,
        kept.iter().any(|c| c.identity_always),
    );
    let mut cancelled = false;
    out.put("\n")?;
    conn.each_row(&select, &mut |vals| {
        if cancel.load(Ordering::Relaxed) {
            cancelled = true;
            return Ok(false);
        }
        let lits: Vec<String> = vals
            .iter()
            .zip(&kept)
            .map(|(v, c)| dump::literal(kind, v, c.binary))
            .collect();
        out.p.rows += 1;
        if let Some(st) = ins.push(&lits) {
            out.statement(&st)?;
            report(&out.p);
        }
        Ok(true)
    })
    .await?;
    if cancelled {
        return Ok(false);
    }
    if let Some(st) = ins.finish() {
        out.statement(&st)?;
    }
    Ok(true)
}

/// A sequence's settings as `CREATE SEQUENCE` options (type, increment, bounds, start, cache,
/// cycle), so a restored one counts as the original did. Each value comes from the catalog and
/// is checked before it reaches the file; none is written when one is missing.
fn sequence_options(r: &[Option<String>]) -> String {
    let num = |i: usize| at(r, i).parse::<i64>().ok();
    let ty = at(r, 5);
    let (Some(inc), Some(min), Some(max), Some(start), Some(cache)) =
        (num(6), num(7), num(8), num(9), num(10))
    else {
        return String::new();
    };
    let ty = match ty.as_str() {
        "smallint" | "integer" | "bigint" => format!(" AS {ty}"),
        _ => String::new(),
    };
    let cycle = if at(r, 11) == "CYCLE" {
        "CYCLE"
    } else {
        "NO CYCLE"
    };
    format!(
        "{ty} INCREMENT BY {inc} MINVALUE {min} MAXVALUE {max} START WITH {start} CACHE {cache} {cycle}"
    )
}

fn read_error(line: u64, e: std::io::Error) -> Error {
    match e.kind() {
        std::io::ErrorKind::InvalidData => {
            Error::Invalid(format!("line {line}: the file is not UTF-8 text"))
        }
        std::io::ErrorKind::UnexpectedEof => Error::Invalid(format!("line {line}: {e}")),
        _ => e.into(),
    }
}

/// A line of the file (see [`dump::read_line`]) that must be UTF-8: the statements go to the
/// server as text.
fn text_line(
    input: &mut (dyn BufRead + Send),
    line: &mut Vec<u8>,
    pending: usize,
    number: u64,
) -> Result<usize> {
    let n = dump::read_line(input, line, pending).map_err(|e| read_error(number, e))?;
    if std::str::from_utf8(line).is_err() {
        return Err(read_error(number, std::io::ErrorKind::InvalidData.into()));
    }
    Ok(n)
}

/// Read a whole SQL file before [`import_sql`] runs any of it: one that is not UTF-8 (a
/// `mysqldump` with raw BLOB bytes, or in latin1) would otherwise stop midway, with the
/// statements before the bad line already applied.
pub fn check_sql_file(input: &mut dyn BufRead) -> Result<()> {
    let mut line = Vec::new();
    let mut number = 0u64;
    loop {
        line.clear();
        number += 1;
        if dump::read_line(input, &mut line, 0).map_err(|e| read_error(number, e))? == 0 {
            return Ok(());
        }
        if std::str::from_utf8(&line).is_err() {
            return Err(Error::Invalid(format!(
                "line {number}: the file is not UTF-8 text, so nothing was imported \
                 (export it as UTF-8, with binary columns as hex: mysqldump --hex-blob)"
            )));
        }
    }
}

/// Run an SQL file, statement by statement, in `ns` (`USE` on MySQL, the `search_path` on
/// PostgreSQL). PostgreSQL's `COPY ... FROM stdin` (as `pg_dump` writes) takes the rows that
/// follow it in the file.
pub async fn import_sql(
    client: &DbClient,
    ns: Option<&str>,
    input: &mut (dyn BufRead + Send),
    opts: &ImportOptions,
    report: Report<'_>,
    cancel: &AtomicBool,
) -> Result<Summary> {
    let kind = client.kind();
    let mut conn = client.transfer_conn().await?;
    let result = import_sql_on(&mut conn, kind, ns, input, opts, &mut *report, cancel).await;
    conn.close().await;
    result
}

async fn import_sql_on(
    conn: &mut Conn,
    kind: DbKind,
    ns: Option<&str>,
    input: &mut (dyn BufRead + Send),
    opts: &ImportOptions,
    report: Report<'_>,
    cancel: &AtomicBool,
) -> Result<Summary> {
    if let Some(ns) = ns.filter(|n| !n.is_empty()) {
        let q = quote_ident(kind, ns);
        conn.run(&match kind {
            DbKind::Mysql => format!("USE {q}"),
            DbKind::Postgres => format!("SET search_path TO {q}"),
        })
        .await?;
    }
    let backslash = kind == DbKind::Mysql && conn.backslash_escapes().await?;
    let atomic = kind == DbKind::Postgres && opts.stop_on_error;
    if atomic {
        conn.run("BEGIN").await?;
    }
    let mut sp = Splitter::new(kind, backslash);
    let mut s = Summary::default();
    let mut line = Vec::new();
    'file: loop {
        line.clear();
        let n = text_line(input, &mut line, sp.pending(), sp.line() + 1)?;
        let statements: Vec<Statement> = if n == 0 {
            sp.finish().into_iter().collect()
        } else {
            s.progress.bytes += n as u64;
            sp.push_line(std::str::from_utf8(&line).expect("checked by text_line"))
        };
        for st in statements {
            if cancel.load(Ordering::Relaxed) {
                s.cancelled = true;
                break 'file;
            }
            // The whole file is one transaction already: its own BEGIN and COMMIT would end
            // it early (and keep what ran so far), so they are skipped; a ROLLBACK stops it.
            if atomic {
                match dump::transaction_control(&st.sql) {
                    Some(false) => {
                        s.progress.statements += 1;
                        continue;
                    }
                    Some(true) => {
                        s.error(
                            st.line,
                            &st.sql,
                            "the file rolls back its transaction: the whole file runs as one, \
                             so nothing is kept"
                                .into(),
                        );
                        break 'file;
                    }
                    None => {}
                }
            }
            let copy = dump::is_copy_from_stdin(&st.sql);
            let outcome = match (copy, &mut *conn) {
                (true, Conn::Postgres(c)) => {
                    copy_in(c, &st.sql, input, &mut sp, &mut s, cancel).await?
                }
                (_, other) => other.exec(&st.sql).await,
            };
            s.progress.statements += 1;
            match outcome {
                Ok(rows) => s.progress.rows += rows,
                Err(e) => {
                    let lost = broken(&e);
                    s.error(st.line, &st.sql, map_err(e).to_string());
                    if opts.stop_on_error || lost {
                        report(&s.progress);
                        break 'file;
                    }
                }
            }
            if s.cancelled {
                break 'file;
            }
            if kind == DbKind::Mysql && dump::sets_sql_mode(&st.sql) {
                sp.set_backslash_escapes(conn.backslash_escapes().await?);
            }
            report(&s.progress);
        }
        if n == 0 {
            break;
        }
    }
    if atomic {
        if s.errors.is_empty() && !s.cancelled {
            conn.run("COMMIT").await?;
        } else {
            // The transaction already failed; the rollback only ends it.
            let _ = conn.exec("ROLLBACK").await;
            s.rolled_back = true;
        }
    }
    report(&s.progress);
    Ok(s)
}

/// Feed the rows after a `COPY ... FROM stdin` to the server, up to the `\.` line. The rows
/// are read to their end even when the server refused the statement, so the file goes on
/// from the next statement.
async fn copy_in(
    c: &mut sqlx::PgConnection,
    sql: &str,
    input: &mut (dyn BufRead + Send),
    sp: &mut Splitter,
    s: &mut Summary,
    cancel: &AtomicBool,
) -> Result<std::result::Result<u64, sqlx::Error>> {
    let mut failed = None;
    let mut copy = match c.copy_in_raw(sql).await {
        Ok(cp) => Some(cp),
        Err(e) => {
            failed = Some(e);
            None
        }
    };
    let mut buf: Vec<u8> = Vec::new();
    let mut line = Vec::new();
    loop {
        if cancel.load(Ordering::Relaxed) {
            s.cancelled = true;
            if let Some(cp) = copy.take() {
                let _ = cp.abort("cancelled").await;
            }
            return Ok(Ok(0));
        }
        line.clear();
        let n = text_line(input, &mut line, 0, sp.line() + 1)?;
        if n == 0 {
            break;
        }
        sp.skip_line(n as u64);
        s.progress.bytes += n as u64;
        if dump::trim_line(&line) == b"\\." {
            break;
        }
        if let Some(cp) = copy.as_mut() {
            buf.extend_from_slice(&line);
            if buf.len() >= 1 << 16 {
                let sent = cp.send(std::mem::take(&mut buf)).await.map(|_| ());
                if let Err(e) = sent {
                    failed = Some(e);
                    copy = None;
                }
            }
        }
    }
    let Some(mut cp) = copy else {
        return Ok(Err(failed.unwrap_or(sqlx::Error::WorkerCrashed)));
    };
    if !buf.is_empty() {
        let sent = cp.send(buf).await.map(|_| ());
        if let Err(e) = sent {
            return Ok(Err(e));
        }
    }
    Ok(cp.finish().await)
}

/// Insert a CSV file's records into `ns.table` in one transaction: all of them, or none
/// when a record is refused or the import is cancelled.
pub async fn import_csv(
    client: &DbClient,
    ns: &str,
    table: &str,
    input: &mut (dyn BufRead + Send),
    opts: &CsvOptions,
    report: Report<'_>,
    cancel: &AtomicBool,
) -> Result<Summary> {
    let kind = client.kind();
    let known = client.columns(ns, table).await?;
    let targets: Vec<(usize, &str)> = opts
        .columns
        .iter()
        .enumerate()
        .filter_map(|(i, c)| c.as_deref().map(|c| (i, c)))
        .collect();
    if targets.is_empty() {
        return Err(Error::Invalid("pick at least one column to fill".into()));
    }
    for (_, c) in &targets {
        if !known.iter().any(|k| k.name == *c) {
            return Err(Error::Invalid(format!("{table} has no column {c}")));
        }
    }
    let mut conn = client.transfer_conn().await?;
    let result = import_csv_on(
        &mut conn,
        kind,
        ns,
        table,
        &targets,
        input,
        opts,
        &mut *report,
        cancel,
    )
    .await;
    conn.close().await;
    result
}

#[allow(clippy::too_many_arguments)]
async fn import_csv_on(
    conn: &mut Conn,
    kind: DbKind,
    ns: &str,
    table: &str,
    targets: &[(usize, &str)],
    input: &mut (dyn BufRead + Send),
    opts: &CsvOptions,
    report: Report<'_>,
    cancel: &AtomicBool,
) -> Result<Summary> {
    let backslash = kind == DbKind::Mysql && conn.backslash_escapes().await?;
    let name = qualified(kind, ns, table);
    let quoted: Vec<String> = targets.iter().map(|(_, c)| quote_ident(kind, c)).collect();
    conn.run(match kind {
        DbKind::Mysql => "START TRANSACTION",
        DbKind::Postgres => "BEGIN",
    })
    .await?;
    let mut s = Summary::default();
    s.progress.table = Some(table.to_string());
    s.progress.tables = 1;
    if opts.replace {
        // DELETE, not TRUNCATE: MySQL commits before a TRUNCATE.
        let st = format!("DELETE FROM {name}");
        if let Err(e) = conn.exec(&st).await {
            s.error(0, &st, map_err(e).to_string());
        }
    }
    let mut reader = CsvReader::new(input, opts.delimiter);
    let mut ins = Inserts::new(&name, &quoted, false);
    let mut first = true;
    let mut batch_line = 0;
    while s.errors.is_empty() {
        if cancel.load(Ordering::Relaxed) {
            s.cancelled = true;
            break;
        }
        let start = reader.line + 1;
        let record = match reader.next_record() {
            Ok(Some(r)) => r,
            Ok(None) => break,
            // Kept as the import's error, so the transaction is rolled back and reported.
            Err(e) => {
                s.error(start, "", read_error(start, e).to_string());
                break;
            }
        };
        if std::mem::take(&mut first) && opts.header {
            continue;
        }
        if record.len() != opts.columns.len() {
            s.error(
                start,
                &record.join(&opts.delimiter.to_string()),
                format!(
                    "expected {} fields, found {}",
                    opts.columns.len(),
                    record.len()
                ),
            );
            break;
        }
        let lits: Vec<String> = targets
            .iter()
            .map(|(i, _)| {
                let v = &record[*i];
                if opts.null.as_deref() == Some(v.as_str()) {
                    "NULL".to_string()
                } else {
                    dump::text_literal(kind, v, backslash)
                }
            })
            .collect();
        if batch_line == 0 {
            batch_line = start;
        }
        if let Some(st) = ins.push(&lits) {
            run_batch(conn, &st, batch_line, &mut s).await;
            batch_line = 0;
            s.progress.bytes = reader.bytes;
            report(&s.progress);
        }
    }
    if s.errors.is_empty() && !s.cancelled {
        if let Some(st) = ins.finish() {
            run_batch(conn, &st, batch_line, &mut s).await;
        }
    }
    s.progress.bytes = reader.bytes;
    if s.errors.is_empty() && !s.cancelled {
        conn.run("COMMIT").await?;
        s.progress.tables_done = 1;
    } else {
        let _ = conn.exec("ROLLBACK").await;
        s.rolled_back = true;
    }
    report(&s.progress);
    Ok(s)
}

async fn run_batch(conn: &mut Conn, st: &str, line: u64, s: &mut Summary) {
    s.progress.statements += 1;
    match conn.exec(st).await {
        Ok(n) => s.progress.rows += n,
        Err(e) => s.error(line, st, map_err(e).to_string()),
    }
}

/// The first records of a CSV file, for the import dialog.
pub fn csv_preview(
    input: impl BufRead,
    delimiter: char,
    records: usize,
) -> Result<Vec<Vec<String>>> {
    let mut r = CsvReader::new(input, delimiter);
    let mut out = Vec::new();
    while out.len() < records {
        match r.next_record().map_err(|e| read_error(r.line + 1, e))? {
            Some(rec) => out.push(rec),
            None => break,
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_that_is_not_utf8_is_refused_before_anything_runs() {
        check_sql_file(&mut &b"SELECT 1;\nSELECT 'caf\xc3\xa9';\n"[..]).unwrap();
        let e = check_sql_file(&mut &b"SELECT 1;\nINSERT INTO t VALUES ('\xff');\n"[..])
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("line 2") && e.contains("nothing was imported"),
            "{e}"
        );
    }
}
