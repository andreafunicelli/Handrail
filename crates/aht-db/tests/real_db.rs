//! Against real servers: set `HANDRAIL_MYSQL_TEST` and/or `HANDRAIL_PG_TEST` to
//! `user:password@host:port/database` (CI starts MySQL 8 and PostgreSQL 16 in containers).
//! Without them every test here passes without doing anything.

use aht_db::{DbClient, DbKind, DbParams, DbTls, QueryLimits, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

fn params(kind: DbKind) -> Option<DbParams> {
    let var = match kind {
        DbKind::Mysql => "HANDRAIL_MYSQL_TEST",
        DbKind::Postgres => "HANDRAIL_PG_TEST",
    };
    let url = std::env::var(var).ok().filter(|v| !v.is_empty())?;
    let (cred, rest) = url.rsplit_once('@').expect("user:password@host:port/db");
    let (user, password) = cred.split_once(':').expect("user:password");
    let (addr, db) = rest.split_once('/').expect("host:port/db");
    let (host, port) = addr.rsplit_once(':').expect("host:port");
    Some(DbParams {
        kind,
        host: host.into(),
        port: port.parse().expect("port"),
        user: user.into(),
        password: Zeroizing::new(password.into()),
        database: Some(db.into()),
        tls: DbTls::Prefer,
        verify_host: false,
        connect_timeout: Duration::from_secs(20),
    })
}

fn text(v: &Value) -> &str {
    match v {
        Value::Text(s) => s,
        other => panic!("expected text, got {other:?}"),
    }
}

async fn each_server(test: impl AsyncFn(DbClient, DbKind)) {
    for kind in [DbKind::Mysql, DbKind::Postgres] {
        if let Some(p) = params(kind) {
            let client = DbClient::connect(&p).await.expect("connect");
            assert_eq!(client.kind(), kind);
            test(client, kind).await;
        }
    }
}

#[tokio::test]
async fn statements_results_and_schema() {
    each_server(async |db, kind| {
        assert!(!db.version().await.unwrap().is_empty());
        let limits = QueryLimits::default();
        let id_type = match kind {
            DbKind::Mysql => "INT AUTO_INCREMENT",
            DbKind::Postgres => "SERIAL",
        };
        let out = db
            .query(
                &format!(
                    "DROP TABLE IF EXISTS hr_people; \
                     CREATE TABLE hr_people (id {id_type} PRIMARY KEY, name VARCHAR(40) NOT NULL, \
                     note TEXT DEFAULT NULL); \
                     INSERT INTO hr_people (name) VALUES ('ada'), ('grace'); \
                     SELECT id, name, note FROM hr_people ORDER BY id"
                ),
                limits,
            )
            .await
            .unwrap();
        assert_eq!(out.error, None, "{out:?}");
        let last = out.results.last().unwrap();
        assert_eq!(
            last.columns
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            ["id", "name", "note"]
        );
        assert_eq!(last.rows.len(), 2);
        assert_eq!(text(&last.rows[1][1]), "grace");
        assert_eq!(last.rows[0][2], Value::Null);
        assert!(out.results.iter().any(|r| r.rows_affected == 2), "{out:?}");

        let ns = match kind {
            DbKind::Mysql => db.current_database().await.unwrap().unwrap(),
            DbKind::Postgres => "public".to_string(),
        };
        assert!(db.namespaces().await.unwrap().contains(&ns));
        let tables = db.tables(&ns).await.unwrap();
        assert!(tables.iter().any(|t| t.name == "hr_people" && !t.view));
        let cols = db.columns(&ns, "hr_people").await.unwrap();
        assert_eq!(cols.len(), 3);
        assert!(cols[0].primary_key && !cols[1].primary_key);
        assert!(!cols[1].nullable && cols[2].nullable);
        assert!(
            cols[1].data_type.to_lowercase().contains("char"),
            "{cols:?}"
        );

        // An error stops the run but keeps what came before it.
        let out = db
            .query("SELECT 1; SELECT * FROM hr_missing; SELECT 2", limits)
            .await
            .unwrap();
        assert!(out.error.is_some());
        assert_eq!(text(&out.results[0].rows[0][0]), "1");
        assert!(!out.results.iter().any(|r| r
            .rows
            .first()
            .is_some_and(|row| row[0] == Value::Text("2".into()))));
        db.close().await;
    })
    .await;
}

#[tokio::test]
async fn session_state_lasts_between_runs() {
    each_server(async |db, kind| {
        let limits = QueryLimits::default();
        let (set, get) = match kind {
            DbKind::Mysql => ("SET @hr_x = 41 + 1", "SELECT @hr_x"),
            DbKind::Postgres => (
                "CREATE TEMP TABLE hr_tmp (x int); INSERT INTO hr_tmp VALUES (42)",
                "SELECT x FROM hr_tmp",
            ),
        };
        assert_eq!(db.query(set, limits).await.unwrap().error, None);
        let out = db.query(get, limits).await.unwrap();
        assert_eq!(text(&out.results[0].rows[0][0]), "42");
    })
    .await;
}

#[tokio::test]
async fn long_results_are_cut_and_the_connection_stays_usable() {
    each_server(async |db, kind| {
        let many = match kind {
            DbKind::Mysql => {
                "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 200000) \
                 SELECT i, REPEAT('x', 50) FROM n"
            }
            DbKind::Postgres => "SELECT i, repeat('x', 50) FROM generate_series(1, 200000) i",
        };
        // Recursion depth is limited per session in MySQL.
        if kind == DbKind::Mysql {
            db.query(
                "SET SESSION cte_max_recursion_depth = 1000000",
                QueryLimits::default(),
            )
            .await
            .unwrap();
        }
        let limits = QueryLimits {
            max_rows: 10,
            max_bytes: 1 << 20,
        };
        let out = db.query(many, limits).await.unwrap();
        assert_eq!(out.error, None, "{out:?}");
        assert_eq!(out.results[0].rows.len(), 10);
        assert!(out.results[0].truncated);
        let again = db.query("SELECT 7", limits).await.unwrap();
        assert_eq!(text(&again.results[0].rows[0][0]), "7");
    })
    .await;
}

#[tokio::test]
async fn a_running_statement_can_be_cancelled() {
    each_server(async |db, kind| {
        let db = Arc::new(db);
        let sleep = match kind {
            DbKind::Mysql => "SELECT SLEEP(30)",
            DbKind::Postgres => "SELECT pg_sleep(30)",
        };
        let started = Instant::now();
        let runner = {
            let db = db.clone();
            tokio::spawn(async move { db.query(sleep, QueryLimits::default()).await })
        };
        tokio::time::sleep(Duration::from_millis(800)).await;
        db.cancel().await.unwrap();
        let out = runner.await.unwrap().unwrap();
        assert!(started.elapsed() < Duration::from_secs(15));
        // MySQL's SLEEP returns 1 when interrupted; PostgreSQL reports the cancel as an error.
        match kind {
            DbKind::Mysql => assert!(
                out.error.is_some() || text(&out.results[0].rows[0][0]) == "1",
                "{out:?}"
            ),
            DbKind::Postgres => assert!(out.error.is_some(), "{out:?}"),
        }
        let again = db.query("SELECT 3", QueryLimits::default()).await.unwrap();
        assert_eq!(text(&again.results[0].rows[0][0]), "3");
    })
    .await;
}

#[tokio::test]
async fn binary_values_are_hex() {
    if let Some(p) = params(DbKind::Mysql) {
        let db = DbClient::connect(&p).await.unwrap();
        let out = db
            .query("SELECT UNHEX('FF00')", QueryLimits::default())
            .await
            .unwrap();
        assert_eq!(
            out.results[0].rows[0][0],
            Value::Bytes { hex: "ff00".into() }
        );
    }
}

#[tokio::test]
async fn wrong_password_is_an_auth_error() {
    for kind in [DbKind::Mysql, DbKind::Postgres] {
        if let Some(mut p) = params(kind) {
            p.password = Zeroizing::new("definitely-wrong".into());
            match DbClient::connect(&p).await {
                Err(aht_db::Error::Auth(_)) => {}
                Err(e) => panic!("{kind:?}: expected an auth error, got {e}"),
                Ok(_) => panic!("{kind:?}: connected with a wrong password"),
            }
        }
    }
}

#[tokio::test]
async fn grid_edits_apply_in_one_transaction() {
    use aht_db::sql::RowEdit;
    each_server(async |db, kind| {
        let limits = QueryLimits::default();
        let ns = match kind {
            DbKind::Mysql => db.current_database().await.unwrap().unwrap(),
            DbKind::Postgres => "public".to_string(),
        };
        let setup = db
            .query(
                "DROP TABLE IF EXISTS hr_edit; \
                 CREATE TABLE hr_edit (id INT PRIMARY KEY, name VARCHAR(20), qty INT); \
                 INSERT INTO hr_edit VALUES (1, 'one', 10), (2, 'two', 20), (3, 'three', 30)",
                limits,
            )
            .await
            .unwrap();
        assert_eq!(setup.error, None);
        let key = |id: &str| vec![("id".to_string(), Some(id.to_string()))];
        let n = db
            .apply_edits(
                &ns,
                "hr_edit",
                &[
                    RowEdit::Update {
                        key: key("1"),
                        set: vec![
                            ("name".into(), Some("O'One".into())),
                            ("qty".into(), Some("11".into())),
                        ],
                    },
                    RowEdit::Delete { key: key("2") },
                    RowEdit::Insert {
                        values: vec![("id".into(), Some("4".into())), ("name".into(), None)],
                    },
                ],
            )
            .await
            .unwrap();
        assert_eq!(n, 3);
        let rows = |out: aht_db::QueryOutput| -> Vec<Vec<Value>> { out.results[0].rows.clone() };
        let after = rows(
            db.query("SELECT id, name, qty FROM hr_edit ORDER BY id", limits)
                .await
                .unwrap(),
        );
        assert_eq!(after.len(), 3);
        assert_eq!(text(&after[0][1]), "O'One");
        assert_eq!(text(&after[0][2]), "11");
        assert_eq!(text(&after[2][0]), "4");
        assert_eq!(after[2][1], Value::Null);

        // A failing statement rolls back the ones before it.
        let err = db
            .apply_edits(
                &ns,
                "hr_edit",
                &[
                    RowEdit::Update {
                        key: key("3"),
                        set: vec![("qty".into(), Some("99".into()))],
                    },
                    RowEdit::Insert {
                        values: vec![("id".into(), Some("1".into()))],
                    },
                ],
            )
            .await;
        assert!(err.is_err());
        let three = rows(
            db.query("SELECT qty FROM hr_edit WHERE id = 3", limits)
                .await
                .unwrap(),
        );
        assert_eq!(text(&three[0][0]), "30");
    })
    .await;
}

#[tokio::test]
async fn agent_reads_cannot_write_and_open_transactions_roll_back() {
    each_server(async |db, _kind| {
        let limits = QueryLimits::default();
        let setup = db
            .query(
                "DROP TABLE IF EXISTS hr_agent; CREATE TABLE hr_agent (id INT PRIMARY KEY)",
                limits,
            )
            .await
            .unwrap();
        assert_eq!(setup.error, None);
        // A write inside the read-only transaction is refused by the server.
        let out = db
            .run_agent("INSERT INTO hr_agent VALUES (1)", limits, true)
            .await
            .unwrap();
        assert!(out.error.is_some(), "{out:?}");
        // A transaction the SQL opens and does not commit is rolled back after the call.
        let out = db
            .run_agent("BEGIN; INSERT INTO hr_agent VALUES (2)", limits, false)
            .await
            .unwrap();
        assert_eq!(out.error, None, "{out:?}");
        // A plain write commits.
        let out = db
            .run_agent("INSERT INTO hr_agent VALUES (3)", limits, false)
            .await
            .unwrap();
        assert_eq!(out.error, None, "{out:?}");
        let out = db
            .run_agent("SELECT id FROM hr_agent ORDER BY id", limits, true)
            .await
            .unwrap();
        assert_eq!(out.error, None, "{out:?}");
        let ids: Vec<&str> = out.results[0].rows.iter().map(|r| text(&r[0])).collect();
        assert_eq!(ids, ["3"]);
    })
    .await;
}

#[tokio::test]
async fn table_pages_info_and_ddl() {
    use aht_db::sql::{select_rows, RowsQuery};
    each_server(async |db, kind| {
        let limits = QueryLimits::default();
        let ns = match kind {
            DbKind::Mysql => db.current_database().await.unwrap().unwrap(),
            DbKind::Postgres => "public".to_string(),
        };
        let setup = db
            .query(
                "DROP VIEW IF EXISTS hr_page_v; DROP TABLE IF EXISTS hr_page; \
                 CREATE TABLE hr_page (id INT PRIMARY KEY, name VARCHAR(20) NOT NULL); \
                 INSERT INTO hr_page VALUES (1, 'a'), (2, 'b'), (3, 'c'), (4, 'd'), (5, 'e'); \
                 CREATE VIEW hr_page_v AS SELECT id FROM hr_page",
                limits,
            )
            .await
            .unwrap();
        assert_eq!(setup.error, None, "{setup:?}");

        // A filtered, sorted page.
        let q = RowsQuery {
            limit: 2,
            offset: 1,
            order: Some(("id", true)),
            filter: Some("id > 1 -- the first row is left out"),
        };
        let out = db
            .query(&select_rows(kind, &ns, "hr_page", &q), limits)
            .await
            .unwrap();
        assert_eq!(out.error, None, "{out:?}");
        let ids: Vec<&str> = out.results[0].rows.iter().map(|r| text(&r[0])).collect();
        assert_eq!(ids, ["4", "3"]);

        let tables = db.tables(&ns).await.unwrap();
        let t = tables.iter().find(|t| t.name == "hr_page").unwrap();
        assert!(!t.view && t.size.is_some(), "{t:?}");
        let v = tables.iter().find(|t| t.name == "hr_page_v").unwrap();
        assert!(v.view && v.size.is_none() && v.rows.is_none(), "{v:?}");
        assert!(!v.materialized && !t.materialized);

        let ddl = db.ddl(&ns, "hr_page").await.unwrap();
        assert!(ddl.starts_with("CREATE TABLE"), "{ddl}");
        assert!(ddl.contains("name") && ddl.contains("PRIMARY KEY"), "{ddl}");
        let ddl = db.ddl(&ns, "hr_page_v").await.unwrap();
        assert!(ddl.contains("VIEW"), "{ddl}");

        // PostgreSQL rebuilds the statement: identity and generated columns keep their kind.
        if kind == DbKind::Postgres {
            let setup = db
                .query(
                    "DROP TABLE IF EXISTS hr_gen; CREATE TABLE hr_gen (\
                     id INT GENERATED ALWAYS AS IDENTITY PRIMARY KEY, \
                     n INT GENERATED BY DEFAULT AS IDENTITY, \
                     price INT NOT NULL DEFAULT 1, \
                     double_price INT GENERATED ALWAYS AS (price * 2) STORED)",
                    limits,
                )
                .await
                .unwrap();
            assert_eq!(setup.error, None, "{setup:?}");
            let ddl = db.ddl(&ns, "hr_gen").await.unwrap();
            assert!(ddl.contains("\"id\" integer GENERATED ALWAYS AS IDENTITY"), "{ddl}");
            assert!(ddl.contains("\"n\" integer GENERATED BY DEFAULT AS IDENTITY"), "{ddl}");
            assert!(ddl.contains("\"price\" integer NOT NULL DEFAULT 1"), "{ddl}");
            assert!(
                ddl.contains("\"double_price\" integer GENERATED ALWAYS AS (") && ddl.contains(") STORED"),
                "{ddl}"
            );
            // The statement it gives runs.
            let again = db
                .query(
                    &format!("DROP TABLE hr_gen; {ddl} INSERT INTO hr_gen (price) VALUES (5); SELECT double_price FROM hr_gen"),
                    limits,
                )
                .await
                .unwrap();
            assert_eq!(again.error, None, "{again:?}");
            assert_eq!(text(&again.results.last().unwrap().rows[0][0]), "10");
        }
    })
    .await;
}

#[tokio::test]
async fn table_structure() {
    each_server(async |db, kind| {
        let limits = QueryLimits::default();
        let ns = match kind {
            DbKind::Mysql => db.current_database().await.unwrap().unwrap(),
            DbKind::Postgres => "public".to_string(),
        };
        let (auto, comment_col, comment_after) = match kind {
            DbKind::Mysql => ("INT AUTO_INCREMENT", " COMMENT 'who bought'", ""),
            DbKind::Postgres => (
                "INT GENERATED BY DEFAULT AS IDENTITY",
                "",
                "; COMMENT ON COLUMN hr_child.owner IS 'who bought'",
            ),
        };
        let setup = db
            .query(
                &format!(
                    "DROP TABLE IF EXISTS hr_child; DROP TABLE IF EXISTS hr_parent; \
                     CREATE TABLE hr_parent (a INT, b INT, PRIMARY KEY (a, b)); \
                     CREATE TABLE hr_child (id {auto} PRIMARY KEY, \
                     owner VARCHAR(40) NOT NULL DEFAULT 'x'{comment_col}, pa INT, pb INT, \
                     CONSTRAINT hr_child_parent FOREIGN KEY (pa, pb) REFERENCES hr_parent (a, b) \
                     ON DELETE CASCADE); \
                     CREATE UNIQUE INDEX hr_child_owner ON hr_child (owner, pa){comment_after}"
                ),
                limits,
            )
            .await
            .unwrap();
        assert_eq!(setup.error, None, "{setup:?}");
        let s = db.structure(&ns, "hr_child").await.unwrap();
        let names: Vec<&str> = s.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["id", "owner", "pa", "pb"]);
        assert!(
            s.columns[0].primary_key && s.columns[0].extra.is_some(),
            "{s:?}"
        );
        let owner = &s.columns[1];
        assert!(
            !owner.nullable && owner.default.as_deref().is_some_and(|d| d.contains('x')),
            "{owner:?}"
        );
        assert_eq!(owner.comment.as_deref(), Some("who bought"));
        if kind == DbKind::Mysql {
            assert!(owner.collation.is_some(), "{owner:?}");
        }

        let pk = s
            .indexes
            .iter()
            .find(|i| i.primary)
            .expect("primary key index");
        assert_eq!(pk.columns, ["id"]);
        let ux = s
            .indexes
            .iter()
            .find(|i| i.name == "hr_child_owner")
            .expect("unique index");
        assert!(ux.unique && !ux.primary);
        assert_eq!(ux.columns, ["owner", "pa"]);

        let fk = &s.foreign_keys[0];
        assert_eq!(fk.name, "hr_child_parent");
        assert_eq!(
            (fk.columns.clone(), fk.ref_table.as_str()),
            (vec!["pa".to_string(), "pb".to_string()], "hr_parent")
        );
        assert_eq!(fk.ref_columns, ["a", "b"]);
        assert_eq!(fk.ref_namespace, ns);
        assert_eq!(fk.on_delete, "CASCADE");
    })
    .await;
}

#[tokio::test]
async fn server_panel_and_kill() {
    use aht_db::ServerInfo;
    for kind in [DbKind::Mysql, DbKind::Postgres] {
        let Some(p) = params(kind) else { continue };
        let db = DbClient::connect(&p).await.unwrap();
        let other = Arc::new(DbClient::connect(&p).await.unwrap());

        let vars = db.server_info(ServerInfo::Variables).await.unwrap();
        assert!(
            vars.rows
                .iter()
                .any(|r| r[0] == Value::Text("max_connections".into())),
            "{kind:?}"
        );
        assert!(!db
            .server_info(ServerInfo::Status)
            .await
            .unwrap()
            .rows
            .is_empty());
        assert!(!db
            .server_info(ServerInfo::Users)
            .await
            .unwrap()
            .rows
            .is_empty());

        // Stop another connection's statement by its process id.
        let sleep = match kind {
            DbKind::Mysql => "SELECT SLEEP(30) AS hr_kill_marker",
            DbKind::Postgres => "SELECT pg_sleep(30) AS hr_kill_marker",
        };
        let started = Instant::now();
        let runner = {
            let other = other.clone();
            tokio::spawn(async move { other.query(sleep, QueryLimits::default()).await })
        };
        let mut pid = None;
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(200)).await;
            let procs = db.server_info(ServerInfo::Processes).await.unwrap();
            pid = procs.rows.iter().find_map(|r| match (&r[0], r.last()) {
                (Value::Text(id), Some(Value::Text(q)))
                    if q.contains("hr_kill_marker")
                        && !q.contains("PROCESSLIST")
                        && !q.contains("pg_stat_activity") =>
                {
                    id.parse::<u64>().ok()
                }
                _ => None,
            });
            if pid.is_some() {
                break;
            }
        }
        let pid = pid.expect("the sleeping statement is listed");
        db.kill(pid, false).await.unwrap();
        let _ = runner.await.unwrap();
        assert!(started.elapsed() < Duration::from_secs(20), "{kind:?}");
    }
}

#[tokio::test]
async fn accounts_and_privileges() {
    use aht_db::accounts::AccountChange;
    for kind in [DbKind::Mysql, DbKind::Postgres] {
        let Some(p) = params(kind) else { continue };
        let db = DbClient::connect(&p).await.unwrap();
        let ns = match kind {
            DbKind::Mysql => db.current_database().await.unwrap().unwrap(),
            DbKind::Postgres => "public".to_string(),
        };
        let user = "hr_acc_user".to_string();
        // A password a careless quoting would break.
        let password = "o'ne\\two\"three".to_string();
        let _ = db
            .account(&AccountChange::Drop {
                user: user.clone(),
                host: None,
            })
            .await;
        db.account(&AccountChange::Create {
            user: user.clone(),
            host: None,
            password: password.clone(),
        })
        .await
        .unwrap();
        db.account(&AccountChange::Grant {
            user: user.clone(),
            host: None,
            privileges: vec!["SELECT".into()],
            namespace: Some(ns.clone()),
        })
        .await
        .unwrap();
        let grants = db.grants(&user, None).await.unwrap();
        let text = format!("{:?}", grants.rows);
        assert!(
            text.contains("SELECT") || matches!(kind, DbKind::Postgres),
            "{kind:?}: {text}"
        );

        // The new account logs in with the password as typed.
        let mut login = p.clone();
        login.user = user.clone();
        login.password = Zeroizing::new(password.clone());
        let as_user = DbClient::connect(&login)
            .await
            .expect("log in as the new account");
        as_user.close().await;

        db.account(&AccountChange::Revoke {
            user: user.clone(),
            host: None,
            privileges: vec!["SELECT".into()],
            namespace: Some(ns.clone()),
        })
        .await
        .unwrap();
        db.account(&AccountChange::Drop {
            user: user.clone(),
            host: None,
        })
        .await
        .unwrap();
        assert!(
            DbClient::connect(&login).await.is_err(),
            "{kind:?}: dropped account still logs in"
        );
    }
}

/// Rows of a query, every cell as text (NULL as "NULL", bytes as `hex:..`).
async fn rows_of(db: &DbClient, sql: &str) -> Vec<Vec<String>> {
    let out = db.query(sql, QueryLimits::default()).await.unwrap();
    assert_eq!(out.error, None, "{sql}: {out:?}");
    out.results
        .last()
        .unwrap()
        .rows
        .iter()
        .map(|r| {
            r.iter()
                .map(|v| match v {
                    Value::Null => "NULL".to_string(),
                    Value::Text(s) => s.clone(),
                    Value::Bytes { hex } => format!("hex:{hex}"),
                })
                .collect()
        })
        .collect()
}

#[tokio::test]
async fn dump_and_import_round_trip() {
    use aht_db::transfer::{self, DumpOptions, ImportOptions};
    use std::sync::atomic::AtomicBool;
    for kind in [DbKind::Mysql, DbKind::Postgres] {
        let Some(p) = params(kind) else { continue };
        let db = DbClient::connect(&p).await.unwrap();
        let ns = "hr_dump_ns";
        let reset = match kind {
            DbKind::Mysql => format!("DROP DATABASE IF EXISTS {ns}; CREATE DATABASE {ns}"),
            DbKind::Postgres => format!("DROP SCHEMA IF EXISTS {ns} CASCADE; CREATE SCHEMA {ns}"),
        };
        let q = |t: &str| format!("{ns}.{t}");
        let schema = match kind {
            DbKind::Mysql => format!(
                "CREATE TABLE {b} (id INT AUTO_INCREMENT PRIMARY KEY, name TEXT, raw BLOB, \
                   twice INT GENERATED ALWAYS AS (CHAR_LENGTH(name) * 2) STORED, \
                   at TIMESTAMP NULL); \
                 CREATE TABLE {a} (id INT PRIMARY KEY, parent INT, \
                   CONSTRAINT a_fk FOREIGN KEY (parent) REFERENCES {b} (id)); \
                 CREATE VIEW {v1} AS SELECT id, name FROM {b}; \
                 CREATE VIEW {v0} AS SELECT id FROM {v1}; \
                 INSERT INTO {b} (name, raw, at) VALUES \
                   ('it''s a \\\\ back\\nslash; -- not a comment', X'00FF27', '2026-01-02 03:04:05'), \
                   ('ünïcode', NULL, NULL), (NULL, X'', NULL); \
                 INSERT INTO {a} VALUES (1, 1), (2, 3)",
                a = q("a_child"),
                b = q("b_parent"),
                v0 = q("a_view"),
                v1 = q("z_view"),
            ),
            DbKind::Postgres => format!(
                "CREATE TABLE {b} (id SERIAL PRIMARY KEY, name TEXT, raw BYTEA, \
                   twice INT GENERATED ALWAYS AS (id * 2) STORED, at TIMESTAMPTZ); \
                 CREATE TABLE {a} (id INT PRIMARY KEY, parent INT, \
                   CONSTRAINT a_fk FOREIGN KEY (parent) REFERENCES {b} (id)); \
                 CREATE TABLE {c} (id INT GENERATED ALWAYS AS IDENTITY PRIMARY KEY, tag TEXT); \
                 CREATE VIEW {v1} AS SELECT id, name FROM {b}; \
                 CREATE VIEW {v0} AS SELECT id FROM {v1}; \
                 INSERT INTO {b} (name, raw, at) VALUES \
                   (E'it''s a \\\\ back\\nslash; -- not a comment', '\\x00ff27', '2026-01-02 03:04:05+00'), \
                   ('ünïcode', NULL, NULL), (NULL, '\\x', NULL); \
                 INSERT INTO {a} VALUES (1, 1), (2, 3); \
                 INSERT INTO {c} (tag) VALUES ('x'), ('y'); \
                 CREATE SEQUENCE {d} INCREMENT BY -1 MINVALUE -100 MAXVALUE -1 START WITH -1; \
                 SELECT nextval('{d}'), nextval('{d}'); \
                 CREATE VIEW {old} AS SELECT 1 AS x; \
                 CREATE VIEW {new} AS SELECT id FROM {b}; \
                 CREATE OR REPLACE VIEW {old} AS SELECT id AS x FROM {new}",
                a = q("a_child"),
                b = q("b_parent"),
                c = q("c_ident"),
                d = q("down_seq"),
                old = q("m_old_view"),
                new = q("n_new_view"),
                v0 = q("a_view"),
                v1 = q("z_view"),
            ),
        };
        let out = db
            .query(&format!("{reset}; {schema}"), QueryLimits::default())
            .await
            .unwrap();
        assert_eq!(out.error, None, "{kind:?}: {out:?}");
        let check = format!(
            "SELECT id, name, raw, twice, at FROM {} ORDER BY id",
            q("b_parent")
        );
        let before = rows_of(&db, &check).await;
        assert_eq!(before.len(), 3);

        let stop = AtomicBool::new(false);
        let mut seen = 0u64;
        let mut report = |_: &transfer::Progress| seen += 1;
        let mut file = Vec::new();
        let opts = DumpOptions {
            tables: Vec::new(),
            structure: true,
            data: true,
            drop: true,
        };
        let sum = transfer::dump(&db, ns, &opts, &mut file, &mut report, &stop)
            .await
            .unwrap();
        assert!(!sum.cancelled);
        assert!(sum.progress.rows >= 5, "{sum:?}");
        let text = String::from_utf8(file.clone()).unwrap();
        assert!(!text.contains("DEFINER="), "{text}");

        // Into an empty namespace, twice (the drops make it repeatable).
        db.query(&reset, QueryLimits::default()).await.unwrap();
        for round in 0..2 {
            let sum = transfer::import_sql(
                &db,
                Some(ns),
                &mut file.as_slice(),
                &ImportOptions {
                    stop_on_error: true,
                },
                &mut report,
                &stop,
            )
            .await
            .unwrap();
            assert!(
                sum.errors.is_empty() && !sum.rolled_back,
                "{kind:?} round {round}: {:?}\n{text}",
                sum.errors
            );
            assert_eq!(rows_of(&db, &check).await, before, "{kind:?}\n{text}");
        }
        assert_eq!(
            rows_of(&db, &format!("SELECT id FROM {} ORDER BY id", q("a_view"))).await,
            vec![vec!["1"], vec!["2"], vec!["3"]]
        );
        // The foreign key came back, and so did the counters.
        let bad = db
            .query(
                &format!("INSERT INTO {} VALUES (9, 99)", q("a_child")),
                QueryLimits::default(),
            )
            .await
            .unwrap();
        assert!(bad.error.is_some(), "{kind:?}: the foreign key is missing");
        let next = rows_of(
            &db,
            &match kind {
                DbKind::Mysql => format!(
                    "INSERT INTO {t} (name) VALUES ('new'); SELECT MAX(id) FROM {t}",
                    t = q("b_parent")
                ),
                DbKind::Postgres => format!(
                    "INSERT INTO {t} (name) VALUES ('new') RETURNING id",
                    t = q("b_parent")
                ),
            },
        )
        .await;
        assert_eq!(next, vec![vec!["4"]], "{kind:?}");
        if kind == DbKind::Postgres {
            let id = rows_of(
                &db,
                &format!(
                    "INSERT INTO {} (tag) VALUES ('z') RETURNING id",
                    q("c_ident")
                ),
            )
            .await;
            assert_eq!(id, vec![vec!["3"]]);
            // A descending sequence keeps its settings and position.
            let down = rows_of(&db, &format!("SELECT nextval('{}')", q("down_seq"))).await;
            assert_eq!(down, vec![vec!["-3"]]);
            // A view replaced to read a newer one is created after it.
            let x = rows_of(&db, &format!("SELECT COUNT(*) FROM {}", q("m_old_view"))).await;
            assert_eq!(x, vec![vec!["4"]]);

            // The file's own BEGIN and COMMIT do not end the import's transaction early.
            let file = format!(
                "BEGIN;\nCREATE TABLE {t} (id INT);\nCOMMIT;\nSELECT 1/0;\n",
                t = q("hr_committed")
            );
            let sum = transfer::import_sql(
                &db,
                Some(ns),
                &mut file.as_bytes(),
                &ImportOptions {
                    stop_on_error: true,
                },
                &mut report,
                &stop,
            )
            .await
            .unwrap();
            assert!(sum.rolled_back && sum.errors[0].line == 4, "{sum:?}");
            let gone = rows_of(
                &db,
                &format!("SELECT to_regclass('{}') IS NULL", q("hr_committed")),
            )
            .await;
            assert_eq!(gone, vec![vec!["t"]]);
        }

        // A refused statement: the PostgreSQL run is rolled back as a whole.
        let broken = format!(
            "CREATE TABLE {t} (id INT);\nINSERT INTO {t} VALUES (1);\nINSERT INTO {t} VALUES ('x');\n",
            t = q("hr_partial")
        );
        let sum = transfer::import_sql(
            &db,
            Some(ns),
            &mut broken.as_bytes(),
            &ImportOptions {
                stop_on_error: true,
            },
            &mut report,
            &stop,
        )
        .await
        .unwrap();
        assert_eq!(sum.errors.len(), 1, "{sum:?}");
        assert_eq!(sum.errors[0].line, 3);
        assert_eq!(sum.rolled_back, kind == DbKind::Postgres);

        // CSV: quotes, delimiters inside them, a NULL marker; a bad record keeps nothing.
        let t = q("hr_csv");
        db.query(
            &format!("CREATE TABLE {t} (id INT PRIMARY KEY, name TEXT, note TEXT)"),
            QueryLimits::default(),
        )
        .await
        .unwrap();
        let csv = "id;name;note\n1;\"Rossi; Mario\";\\N\n2;\"say \"\"hi\"\"\";a\\b\n";
        let copts = transfer::CsvOptions {
            delimiter: ';',
            header: true,
            null: Some("\\N".into()),
            columns: vec![Some("id".into()), Some("name".into()), Some("note".into())],
            replace: false,
        };
        let sum = transfer::import_csv(
            &db,
            ns,
            "hr_csv",
            &mut csv.as_bytes(),
            &copts,
            &mut report,
            &stop,
        )
        .await
        .unwrap();
        assert!(sum.errors.is_empty(), "{sum:?}");
        assert_eq!(sum.progress.rows, 2);
        assert_eq!(
            rows_of(&db, &format!("SELECT id, name, note FROM {t} ORDER BY id")).await,
            vec![
                vec!["1", "Rossi; Mario", "NULL"],
                vec!["2", "say \"hi\"", "a\\b"]
            ]
        );
        let bad_csv = "id;name;note\n3;x;y\n4;only two\n";
        let sum = transfer::import_csv(
            &db,
            ns,
            "hr_csv",
            &mut bad_csv.as_bytes(),
            &copts,
            &mut report,
            &stop,
        )
        .await
        .unwrap();
        assert!(sum.rolled_back && sum.errors[0].line == 3, "{sum:?}");
        // A file cut inside a quoted field is refused, not imported half.
        let cut = "id;name;note\n7;\"never closed\n";
        let sum = transfer::import_csv(
            &db,
            ns,
            "hr_csv",
            &mut cut.as_bytes(),
            &copts,
            &mut report,
            &stop,
        )
        .await
        .unwrap();
        assert!(sum.rolled_back && !sum.errors.is_empty(), "{sum:?}");
        assert_eq!(
            rows_of(&db, &format!("SELECT COUNT(*) FROM {t}")).await,
            vec![vec!["2"]]
        );
        assert!(seen > 0);
        let drop = match kind {
            DbKind::Mysql => format!("DROP DATABASE {ns}"),
            DbKind::Postgres => format!("DROP SCHEMA {ns} CASCADE"),
        };
        db.query(&drop, QueryLimits::default()).await.unwrap();
        db.close().await;
    }
}
