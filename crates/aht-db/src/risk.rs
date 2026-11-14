//! How risky SQL written by an agent is: what decides whether it runs at once or waits for the
//! user's approval.
//!
//! The text is split into statements by a small lexer that skips strings, quoted identifiers and
//! comments the way the server does. Where servers differ by configuration (backslash escapes in
//! strings: MySQL's `NO_BACKSLASH_ESCAPES`, PostgreSQL's `standard_conforming_strings`), the text
//! is read both ways and the riskier reading wins, so a statement cannot hide inside what one
//! reading takes for a string. Reads also run in a read-only transaction (see
//! [`crate::DbClient::run_agent`]), so a misclassified read that writes fails on the server.

use crate::DbKind;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SqlRisk {
    /// Only reads data: `SELECT`, `SHOW`, `EXPLAIN`, `DESCRIBE`, `WITH` without data changes.
    Read,
    /// Changes data or schema, or anything not known to be a read.
    Write,
    /// `DROP`, `TRUNCATE`, `DELETE`/`UPDATE` without `WHERE`, `ALTER ... DROP`, `GRANT`,
    /// `REVOKE`, `COPY ... PROGRAM`, MySQL executable comments (`/*! ... */`, `/*M! ... */`).
    Destructive,
}

/// The risk of `sql` (one or more statements) and how many statements it holds.
pub fn classify(kind: DbKind, sql: &str) -> (SqlRisk, usize) {
    let a = classify_with(kind, sql, true);
    let b = classify_with(kind, sql, false);
    (a.0.max(b.0), a.1.max(b.1))
}

fn classify_with(kind: DbKind, sql: &str, backslash_escapes: bool) -> (SqlRisk, usize) {
    let lexed = lex(kind, sql, backslash_escapes);
    let mut risk = if lexed.executable_comment {
        SqlRisk::Destructive
    } else {
        SqlRisk::Read
    };
    if lexed.unterminated {
        // An unclosed string or comment: the server will reject it, or read it otherwise.
        risk = risk.max(SqlRisk::Write);
    }
    let statements: Vec<&[Tok]> = lexed
        .tokens
        .split(|t| *t == Tok::Semi)
        .filter(|s| !s.is_empty())
        .collect();
    for st in &statements {
        risk = risk.max(statement_risk(st));
    }
    (risk, statements.len())
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    /// A keyword or bare identifier, upper-cased.
    Word(String),
    Open,
    Close,
    Semi,
    /// A quoted name: `"name"`, and `` `name` `` on MySQL.
    Quoted,
    /// Anything else that is code (operators, numbers, quoted text).
    Other,
}

struct Lexed {
    tokens: Vec<Tok>,
    executable_comment: bool,
    unterminated: bool,
}

fn lex(kind: DbKind, sql: &str, backslash_escapes: bool) -> Lexed {
    let c: Vec<char> = sql.chars().collect();
    let mut out = Lexed {
        tokens: Vec::new(),
        executable_comment: false,
        unterminated: false,
    };
    let mysql = kind == DbKind::Mysql;
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        let next = c.get(i + 1).copied();
        // Comments.
        if ch == '-' && next == Some('-') {
            // MySQL needs an ASCII space or control character (or the end) after `--`: `1--1`
            // is arithmetic there, and so is `--` before a no-break space.
            let after = c.get(i + 2).copied();
            if !mysql || after.is_none_or(|a| a == ' ' || a.is_ascii_control()) {
                i = line_end(&c, i);
                continue;
            }
        }
        if mysql && ch == '#' {
            i = line_end(&c, i);
            continue;
        }
        if ch == '/' && next == Some('*') {
            // MySQL runs the inside of `/*! ... */` as SQL, MariaDB also `/*M! ... */`.
            if mysql
                && (c.get(i + 2) == Some(&'!')
                    || (matches!(c.get(i + 2), Some('M' | 'm')) && c.get(i + 3) == Some(&'!')))
            {
                out.executable_comment = true;
            }
            // PostgreSQL nests block comments; MySQL does not.
            let mut depth = 1;
            i += 2;
            while i < c.len() && depth > 0 {
                if c[i] == '*' && c.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                } else if !mysql && c[i] == '/' && c.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if depth > 0 {
                out.unterminated = true;
            }
            continue;
        }
        // Quoted text and names.
        if ch == '\'' || ch == '"' || (mysql && ch == '`') {
            // PostgreSQL: `E'...'` strings take backslash escapes whatever the setting.
            let escape_string = !mysql
                && ch == '\''
                && i > 0
                && matches!(c[i - 1], 'e' | 'E')
                && (i < 2 || !is_word_char(c[i - 2]));
            let escapes =
                ch != '`' && (backslash_escapes || escape_string) && (mysql || ch == '\'');
            i += 1;
            let mut closed = false;
            while i < c.len() {
                if escapes && c[i] == '\\' {
                    i += 2;
                    continue;
                }
                if c[i] == ch {
                    if c.get(i + 1) == Some(&ch) {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    closed = true;
                    break;
                }
                i += 1;
            }
            if !closed {
                out.unterminated = true;
            }
            out.tokens
                .push(if ch == '\'' { Tok::Other } else { Tok::Quoted });
            continue;
        }
        // PostgreSQL dollar quoting: `$tag$ ... $tag$` (but `$1` is a parameter).
        if !mysql && ch == '$' && !c.get(i + 1).is_some_and(|d| d.is_ascii_digit()) {
            let mut j = i + 1;
            while j < c.len() && (c[j].is_alphanumeric() || c[j] == '_') {
                j += 1;
            }
            if c.get(j) == Some(&'$') {
                let tag: Vec<char> = c[i..=j].to_vec();
                let mut k = j + 1;
                let mut closed = false;
                while k < c.len() {
                    if c[k..].starts_with(&tag) {
                        k += tag.len();
                        closed = true;
                        break;
                    }
                    k += 1;
                }
                if !closed {
                    out.unterminated = true;
                }
                out.tokens.push(Tok::Other);
                i = k;
                continue;
            }
        }
        match ch {
            ';' => out.tokens.push(Tok::Semi),
            '(' => out.tokens.push(Tok::Open),
            ')' => out.tokens.push(Tok::Close),
            c0 if c0.is_whitespace() => {}
            c0 if is_word_char(c0) && !c0.is_ascii_digit() => {
                let start = i;
                while i < c.len() && is_word_char(c[i]) {
                    i += 1;
                }
                let w: String = c[start..i].iter().collect();
                out.tokens.push(Tok::Word(w.to_uppercase()));
                continue;
            }
            _ => out.tokens.push(Tok::Other),
        }
        i += 1;
    }
    out
}

/// Where a `--` or `#` comment starting at `i` ends. PostgreSQL ends it at a carriage return
/// as well as a newline; ending it early on MySQL too only shows more text as code.
fn line_end(c: &[char], mut i: usize) -> usize {
    while i < c.len() && c[i] != '\n' && c[i] != '\r' {
        i += 1;
    }
    i
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

fn statement_risk(st: &[Tok]) -> SqlRisk {
    let words: Vec<&str> = st
        .iter()
        .filter_map(|t| match t {
            Tok::Word(w) => Some(w.as_str()),
            _ => None,
        })
        .collect();
    let has = |w: &str| words.contains(&w);
    let first = match st.first() {
        Some(Tok::Word(w)) => w.as_str(),
        // `(SELECT ...)` and anything else unusual.
        _ => return SqlRisk::Write,
    };
    let changes_data = || {
        // (Not REPLACE: inside a query it is the string function.)
        ["INSERT", "UPDATE", "DELETE", "MERGE", "INTO"]
            .iter()
            .any(|w| has(w))
    };
    let locks = || {
        // SELECT ... FOR UPDATE / FOR SHARE / LOCK IN SHARE MODE take row locks.
        words
            .windows(2)
            .any(|p| p[0] == "FOR" && matches!(p[1], "UPDATE" | "SHARE" | "NO" | "KEY"))
            || has("LOCK")
    };
    let analyze = has("ANALYZE") || has("ANALYSE");
    let base = match first {
        "SELECT" | "WITH" | "VALUES" | "TABLE" => {
            if changes_data() || locks() {
                SqlRisk::Write
            } else {
                SqlRisk::Read
            }
        }
        "SHOW" | "DESCRIBE" | "DESC" | "HELP" => SqlRisk::Read,
        // EXPLAIN ANALYZE runs the statement.
        "EXPLAIN" if analyze => SqlRisk::Write,
        "EXPLAIN" => return SqlRisk::Read,
        "DROP" | "TRUNCATE" | "GRANT" | "REVOKE" => SqlRisk::Destructive,
        "ALTER" if has("DROP") => SqlRisk::Destructive,
        "COPY" if has("PROGRAM") => SqlRisk::Destructive,
        _ => SqlRisk::Write,
    };
    let base = if calls_side_effect(st) {
        base.max(SqlRisk::Write)
    } else {
        base
    };
    // A DELETE or UPDATE without its own WHERE changes every row, wherever it sits: at the
    // start, after WITH, inside a WITH clause, or under EXPLAIN ANALYZE.
    if unqualified_change(st) {
        SqlRisk::Destructive
    } else {
        base
    }
}

/// Functions that act beyond reading the data, even in a read-only transaction: they stop
/// other sessions, read or write server files, reach other servers, take locks, change
/// settings or sequences, run SQL given as a string, or just hold the connection.
const SIDE_EFFECTS: &[&str] = &[
    // PostgreSQL
    "PG_TERMINATE_BACKEND",
    "PG_CANCEL_BACKEND",
    "PG_READ_FILE",
    "PG_READ_BINARY_FILE",
    "PG_STAT_FILE",
    "PG_FILE_WRITE",
    "PG_FILE_RENAME",
    "PG_FILE_UNLINK",
    "PG_FILE_SYNC",
    "PG_PROMOTE",
    "PG_RELOAD_CONF",
    "PG_ROTATE_LOGFILE",
    "PG_SWITCH_WAL",
    "PG_CREATE_RESTORE_POINT",
    "PG_BACKUP_START",
    "PG_BACKUP_STOP",
    "PG_START_BACKUP",
    "PG_STOP_BACKUP",
    "PG_WAL_REPLAY_PAUSE",
    "PG_WAL_REPLAY_RESUME",
    "PG_CREATE_PHYSICAL_REPLICATION_SLOT",
    "PG_CREATE_LOGICAL_REPLICATION_SLOT",
    "PG_DROP_REPLICATION_SLOT",
    "PG_REPLICATION_SLOT_ADVANCE",
    "PG_LOGICAL_SLOT_GET_CHANGES",
    "PG_LOGICAL_SLOT_GET_BINARY_CHANGES",
    "PG_LOGICAL_EMIT_MESSAGE",
    "PG_LOG_BACKEND_MEMORY_CONTEXTS",
    "PG_NOTIFY",
    "PG_SLEEP",
    "PG_SLEEP_FOR",
    "PG_SLEEP_UNTIL",
    "SET_CONFIG",
    "NEXTVAL",
    "SETVAL",
    "CURSOR_TO_XML",
    "TS_STAT",
    // MySQL / MariaDB
    "LOAD_FILE",
    "GET_LOCK",
    "SLEEP",
    "BENCHMARK",
    "MASTER_POS_WAIT",
    "SOURCE_POS_WAIT",
    "WAIT_FOR_EXECUTED_GTID_SET",
    "WAIT_UNTIL_SQL_THREAD_AFTER_GTIDS",
    "SYS_EXEC",
    "SYS_EVAL",
];

/// Families of such functions, by prefix.
const SIDE_EFFECT_PREFIXES: &[&str] = &[
    "PG_ADVISORY_",
    "PG_TRY_ADVISORY_",
    "PG_LS_",
    "PG_STAT_RESET",
    "LO_",
    "DBLINK",
    "QUERY_TO_XML",
];

/// Whether `st` calls one of [`SIDE_EFFECTS`]. A quoted name called as a function cannot be
/// checked against the list, so it counts too.
fn calls_side_effect(st: &[Tok]) -> bool {
    st.windows(2).any(|p| match (&p[0], &p[1]) {
        (Tok::Word(w), Tok::Open) => {
            SIDE_EFFECTS.contains(&w.as_str())
                || SIDE_EFFECT_PREFIXES.iter().any(|f| w.starts_with(f))
        }
        (Tok::Quoted, Tok::Open) => true,
        _ => false,
    })
}

/// Whether `st` holds a DELETE or UPDATE with no WHERE at its own level (one in a subquery
/// does not count).
fn unqualified_change(st: &[Tok]) -> bool {
    // Parenthesis depth of each token (a closing one counts as outside).
    let mut depth = 0i32;
    let depths: Vec<i32> = st
        .iter()
        .map(|t| {
            if *t == Tok::Close {
                depth -= 1;
            }
            let d = depth;
            if *t == Tok::Open {
                depth += 1;
            }
            d
        })
        .collect();
    let word = |i: usize| match st.get(i) {
        Some(Tok::Word(w)) => Some(w.as_str()),
        _ => None,
    };
    (0..st.len()).any(|i| {
        if !matches!(word(i), Some("DELETE" | "UPDATE")) {
            return false;
        }
        // Not a statement: ON DELETE/UPDATE (foreign keys), FOR [NO KEY] UPDATE, ON DUPLICATE
        // KEY UPDATE, DO UPDATE, MERGE's THEN UPDATE, triggers' BEFORE/AFTER/OR/OF UPDATE.
        if i > 0
            && matches!(
                word(i - 1),
                Some(
                    "ON" | "FOR"
                        | "KEY"
                        | "DO"
                        | "THEN"
                        | "BEFORE"
                        | "AFTER"
                        | "OR"
                        | "OF"
                        | "INSTEAD"
                        | "GRANT"
                )
            )
        {
            return false;
        }
        let d = depths[i];
        !st[i + 1..]
            .iter()
            .zip(&depths[i + 1..])
            .take_while(|(_, dd)| **dd >= d)
            .any(|(t, dd)| *dd == d && matches!(t, Tok::Word(w) if w == "WHERE"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use DbKind::{Mysql, Postgres};
    use SqlRisk::{Destructive, Read, Write};

    fn risk(kind: DbKind, sql: &str) -> SqlRisk {
        classify(kind, sql).0
    }

    #[test]
    fn reads() {
        for sql in [
            "SELECT * FROM t WHERE a = 'x; DROP TABLE t'",
            "  select 1; -- DROP TABLE t\n SELECT 2",
            "SHOW TABLES",
            "EXPLAIN SELECT * FROM t",
            "EXPLAIN DELETE FROM t",
            "WITH x AS (SELECT 1) SELECT * FROM x",
            "DESCRIBE t",
            "/* DROP TABLE t */ SELECT 1",
            "",
        ] {
            assert_eq!(risk(Mysql, sql), Read, "{sql}");
            assert_eq!(risk(Postgres, sql), Read, "{sql}");
        }
        assert_eq!(
            risk(Postgres, "SELECT $$ ; DROP TABLE t; $$, $tag$x$tag$"),
            Read
        );
        assert_eq!(risk(Postgres, "SELECT \"a;b\" FROM t"), Read);
        assert_eq!(risk(Mysql, "SELECT REPLACE(name, 'a', 'b') FROM t"), Read);
        assert_eq!(risk(Postgres, "SELECT $1, a$b FROM t"), Read);
        assert_eq!(classify(Mysql, "SELECT 1; SELECT 2;").1, 2);
    }

    #[test]
    fn writes() {
        for sql in [
            "INSERT INTO t VALUES (1)",
            "UPDATE t SET a = 1 WHERE id = 2",
            "DELETE FROM t WHERE id = 2",
            "SELECT 1; INSERT INTO t VALUES (1)",
            "WITH d AS (DELETE FROM t WHERE a = 1 RETURNING *) SELECT * FROM d",
            "SELECT * INTO new_t FROM t",
            "SELECT * FROM t FOR UPDATE",
            "EXPLAIN ANALYZE DELETE FROM t WHERE a = 1",
            "INSERT INTO t VALUES (1) ON DUPLICATE KEY UPDATE a = 1",
            "INSERT INTO t VALUES (1) ON CONFLICT (id) DO UPDATE SET a = 1",
            "CREATE TABLE c (p int REFERENCES t ON DELETE CASCADE ON UPDATE CASCADE)",
            "DELETE FROM t WHERE id IN (SELECT id FROM u)",
            "CREATE TABLE x (a int)",
            "BEGIN",
            "SET search_path = x",
            "CALL p()",
            "(SELECT 1)",
            "SELECT 'unterminated",
        ] {
            assert_eq!(risk(Mysql, sql), Write, "{sql}");
            assert_eq!(risk(Postgres, sql), Write, "{sql}");
        }
    }

    #[test]
    fn destructive() {
        for sql in [
            "DROP TABLE t",
            "truncate t",
            "DELETE FROM t",
            "UPDATE t SET a = (SELECT b FROM u WHERE u.id = 1)",
            "ALTER TABLE t DROP COLUMN a",
            "SELECT 1; DROP TABLE t",
            "GRANT ALL ON t TO bob",
            "WITH x AS (SELECT 1) DELETE FROM t",
            "WITH d AS (DELETE FROM t RETURNING *) SELECT count(*) FROM d",
            "WITH d AS (UPDATE t SET a = 1 RETURNING *) SELECT * FROM d WHERE a = 1",
            "EXPLAIN ANALYZE DELETE FROM t",
            "DELETE FROM t; SELECT 1 WHERE true",
        ] {
            assert_eq!(risk(Mysql, sql), Destructive, "{sql}");
            assert_eq!(risk(Postgres, sql), Destructive, "{sql}");
        }
        assert_eq!(risk(Postgres, "COPY t TO PROGRAM 'rm -rf /'"), Destructive);
    }

    #[test]
    fn statements_cannot_hide() {
        // MySQL runs `/*! ... */`.
        assert_eq!(risk(Mysql, "SELECT 1 /*! ; DROP TABLE t */"), Destructive);
        // `--1` is not a comment on MySQL.
        assert_eq!(risk(Mysql, "SELECT 1--1\n; DROP TABLE t"), Destructive);
        assert_eq!(risk(Mysql, "SELECT 1--1; DROP TABLE t"), Destructive);
        // With NO_BACKSLASH_ESCAPES the string ends at the second quote.
        assert_eq!(risk(Mysql, r"SELECT '\'; DROP TABLE t; -- '"), Destructive);
        // PostgreSQL: backslash escapes only in E'' strings, but read both ways.
        assert_eq!(
            risk(Postgres, r"SELECT '\'; DROP TABLE t; -- '"),
            Destructive
        );
        // In an E'' string the quote is escaped whatever the setting: all one string.
        assert_eq!(risk(Postgres, r"SELECT E'\'; DROP TABLE t; -- '"), Read);
        assert_eq!(
            risk(Postgres, r"SELECT E'\'', 1; DROP TABLE t"),
            Destructive
        );
        // Nested comments on PostgreSQL; on MySQL the first `*/` ends the comment.
        assert_eq!(risk(Postgres, "/* /* */ DROP TABLE t; */ SELECT 1"), Read);
        assert_eq!(
            risk(Mysql, "/* /* */ DROP TABLE t; */ SELECT 1"),
            Destructive
        );
        // `#` starts a comment only on MySQL.
        assert_eq!(risk(Postgres, "SELECT 1 # 2; DROP TABLE t"), Destructive);
        assert_eq!(risk(Mysql, "SELECT 1 # ; DROP TABLE t"), Read);
        // PostgreSQL also ends a line comment at a carriage return.
        assert_eq!(
            classify(Postgres, "SELECT 1 -- x\r; COMMIT; DROP TABLE t").0,
            Destructive
        );
        assert_eq!(risk(Mysql, "SELECT 1 # x\r; DROP TABLE t"), Destructive);
        // MariaDB runs `/*M! ... */` too.
        assert_eq!(
            risk(Mysql, "SELECT 1 /*M! ; COMMIT; DROP TABLE t; SELECT 1 */"),
            Destructive
        );
        assert_eq!(risk(Mysql, "SELECT 1 /*M!100000 SLEEP(1) */"), Destructive);
        // `--` before a no-break space is not a comment on MySQL.
        assert_eq!(risk(Mysql, "SELECT 1 --\u{a0}; DROP TABLE t"), Destructive);
        assert_eq!(risk(Mysql, "SELECT 1 --\t; DROP TABLE t"), Read);
    }

    #[test]
    fn side_effect_functions_are_not_reads() {
        for sql in [
            "SELECT pg_terminate_backend(123)",
            "select Pg_Cancel_Backend (123)",
            "SELECT pg_read_file('/etc/passwd')",
            "SELECT pg_read_binary_file('x')",
            "SELECT * FROM pg_ls_dir('.')",
            "SELECT lo_export(1, '/tmp/x')",
            "SELECT lo_import('/etc/passwd')",
            "SELECT * FROM dblink_exec('host=x', 'DROP TABLE t')",
            "SELECT * FROM dblink('host=x', 'SELECT 1') AS t(a int)",
            "SELECT pg_promote()",
            "SELECT pg_reload_conf()",
            "SELECT pg_catalog.pg_sleep(100)",
            "SELECT set_config('search_path', 'x', false)",
            "SELECT nextval('s')",
            "SELECT setval('s', 1)",
            "SELECT pg_advisory_lock(1)",
            "SELECT pg_try_advisory_xact_lock(1)",
            "SELECT pg_sleep/**/(100)",
            "SELECT \"pg_sleep\"(100)",
            "SELECT query_to_xml('SELECT pg_sleep(1)', true, true, '')",
            "WITH x AS (SELECT pg_sleep(1)) SELECT * FROM x",
        ] {
            assert_eq!(risk(Postgres, sql), Write, "{sql}");
        }
        for sql in [
            "SELECT LOAD_FILE('/etc/passwd')",
            "SELECT GET_LOCK('x', 10)",
            "SELECT SLEEP(100)",
            "SELECT BENCHMARK(100000000, MD5('x'))",
            "SELECT * FROM t INTO OUTFILE '/tmp/x'",
            "SELECT * FROM t INTO DUMPFILE '/tmp/x'",
            "SELECT * FROM t FOR SHARE",
            "SELECT * FROM t LOCK IN SHARE MODE",
            "SELECT `sleep`(100)",
        ] {
            assert_eq!(risk(Mysql, sql), Write, "{sql}");
        }
        // Names in strings or as columns are not calls.
        assert_eq!(risk(Postgres, "SELECT 'pg_sleep(1)', sleep FROM t"), Read);
        assert_eq!(risk(Mysql, "SELECT `sleep` FROM t"), Read);
        assert_eq!(risk(Postgres, "SELECT count(*), lower(a) FROM t"), Read);
    }
}
