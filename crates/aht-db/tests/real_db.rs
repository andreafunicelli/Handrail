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
