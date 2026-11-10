//! The parts of export and import that touch no connection: SQL literals for a dump, a
//! splitter that cuts an SQL file read line by line into statements, and a CSV reader.

use crate::{DbKind, Value};
use std::io::BufRead;

/// A string as an SQL literal. MySQL with backslash escapes (the default `sql_mode`, and the
/// one a dump's header sets) escapes the bytes `mysqldump` escapes; without them, and on
/// PostgreSQL (`standard_conforming_strings` on), only quotes are doubled.
pub fn text_literal(kind: DbKind, s: &str, backslash_escapes: bool) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        match ch {
            '\'' => out.push_str("''"),
            '\\' if kind == DbKind::Mysql && backslash_escapes => out.push_str("\\\\"),
            '\0' if kind == DbKind::Mysql && backslash_escapes => out.push_str("\\0"),
            '\n' if kind == DbKind::Mysql && backslash_escapes => out.push_str("\\n"),
            '\r' if kind == DbKind::Mysql && backslash_escapes => out.push_str("\\r"),
            '\u{1a}' if kind == DbKind::Mysql && backslash_escapes => out.push_str("\\Z"),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// Bytes as a literal: `X'..'` on MySQL, a `bytea` on PostgreSQL.
fn bytes_literal(kind: DbKind, hex: &str) -> String {
    match kind {
        DbKind::Mysql => format!("X'{hex}'"),
        DbKind::Postgres => format!("'\\x{hex}'::bytea"),
    }
}

/// A value read over the text protocol as a literal of a dump (MySQL: backslash escapes on).
/// `binary` columns (BLOB, BINARY, BIT, spatial types) are written as hex whatever their bytes.
pub fn literal(kind: DbKind, v: &Value, binary: bool) -> String {
    match v {
        Value::Null => "NULL".into(),
        Value::Bytes { hex } => bytes_literal(kind, hex),
        Value::Text(s) if binary && kind == DbKind::Mysql => {
            bytes_literal(kind, &hex::encode(s.as_bytes()))
        }
        Value::Text(s) => text_literal(kind, s, true),
    }
}

/// Whether a MySQL `DATA_TYPE` holds bytes rather than text.
pub fn mysql_binary_type(data_type: &str) -> bool {
    matches!(
        data_type.to_ascii_lowercase().as_str(),
        "binary"
            | "varbinary"
            | "tinyblob"
            | "blob"
            | "mediumblob"
            | "longblob"
            | "bit"
            | "geometry"
            | "point"
            | "linestring"
            | "polygon"
            | "multipoint"
            | "multilinestring"
            | "multipolygon"
            | "geometrycollection"
            | "geomcollection"
    )
}

/// Rows of one table gathered into multi-row `INSERT`s of about [`Inserts::LIMIT`] bytes.
pub struct Inserts {
    prefix: String,
    buf: String,
    rows: usize,
}

impl Inserts {
    /// Bytes per statement: far below the smallest default `max_allowed_packet` (4 MB).
    pub const LIMIT: usize = 1 << 20;

    /// `table` and `columns` already quoted.
    pub fn new(table: &str, columns: &[String], overriding: bool) -> Inserts {
        Inserts {
            prefix: format!(
                "INSERT INTO {table} ({}){} VALUES\n",
                columns.join(", "),
                if overriding {
                    " OVERRIDING SYSTEM VALUE"
                } else {
                    ""
                }
            ),
            buf: String::new(),
            rows: 0,
        }
    }

    /// Add a row of literals; returns a full statement once the batch is large enough.
    pub fn push(&mut self, literals: &[String]) -> Option<String> {
        if self.rows == 0 {
            self.buf.push_str(&self.prefix);
        } else {
            self.buf.push_str(",\n");
        }
        self.buf.push('(');
        self.buf.push_str(&literals.join(", "));
        self.buf.push(')');
        self.rows += 1;
        (self.buf.len() >= Self::LIMIT).then(|| self.take())
    }

    /// What is left, as a statement (`None` when no row is waiting).
    pub fn finish(&mut self) -> Option<String> {
        (self.rows > 0).then(|| self.take())
    }

    fn take(&mut self) -> String {
        self.rows = 0;
        self.buf.push_str(";\n");
        std::mem::take(&mut self.buf)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum State {
    Normal,
    /// In a quoted string or identifier: the quote, and whether a backslash escapes.
    Quote(u8, bool),
    /// In a block comment, nested this deep (PostgreSQL nests them).
    Block(u32),
    /// In a PostgreSQL dollar-quoted string: the whole tag (`$$`, `$body$`).
    Dollar(Vec<u8>),
}

/// A statement cut from a file, with the line it starts on (1-based).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Statement {
    pub sql: String,
    pub line: u64,
}

/// A statement as the file's bytes (a dump may hold bytes that are not UTF-8: `mysqldump`
/// writes BLOBs raw), with its first line and the byte offset of its first token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawStatement {
    pub sql: Vec<u8>,
    pub line: u64,
    pub offset: u64,
}

/// Cuts SQL into statements as lines arrive, so a file of any size is read a line at a time.
/// It knows strings, quoted identifiers, comments, PostgreSQL dollar quotes, the MySQL
/// client's `DELIMITER` command and psql's backslash commands (`\connect`, `\restrict`: a
/// line of their own, skipped); what a statement means is left to the server.
pub struct Splitter {
    kind: DbKind,
    backslash: bool,
    delimiter: Vec<u8>,
    cur: Vec<u8>,
    /// Where the statement's first token starts in `cur` (comments before it are dropped).
    start: Option<usize>,
    start_line: u64,
    start_offset: u64,
    line: u64,
    /// Byte offset of the next line in the input.
    offset: u64,
    state: State,
}

fn ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

/// The `$tag$` opening at `i`, if one does (`$1` is a parameter, not a tag).
fn dollar_tag(bytes: &[u8], i: usize) -> Option<usize> {
    if i > 0 && (ident_byte(bytes[i - 1]) || bytes[i - 1] == b'$') {
        return None;
    }
    let mut j = i + 1;
    if j < bytes.len() && bytes[j].is_ascii_digit() {
        return None;
    }
    while j < bytes.len()
        && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_' || bytes[j] >= 0x80)
    {
        j += 1;
    }
    (j < bytes.len() && bytes[j] == b'$').then_some(j + 1 - i)
}

/// A line without its line break.
pub fn trim_line(b: &[u8]) -> &[u8] {
    let n = b
        .iter()
        .rev()
        .take_while(|c| matches!(c, b'\r' | b'\n'))
        .count();
    &b[..b.len() - n]
}

fn trim_start(b: &[u8]) -> &[u8] {
    let n = b.iter().take_while(|c| c.is_ascii_whitespace()).count();
    &b[n..]
}

fn trim_end(b: &[u8]) -> &[u8] {
    let n = b
        .iter()
        .rev()
        .take_while(|c| c.is_ascii_whitespace())
        .count();
    &b[..b.len() - n]
}

impl Splitter {
    /// `backslash_escapes`: MySQL strings treat a backslash as an escape (the server's
    /// `sql_mode` lacks `NO_BACKSLASH_ESCAPES`). PostgreSQL only escapes in `E'...'`.
    pub fn new(kind: DbKind, backslash_escapes: bool) -> Splitter {
        Splitter::at(kind, backslash_escapes, 0, 0)
    }

    /// A splitter that starts reading at byte `offset`, on line `line` + 1, between
    /// statements (to read a part of a file again).
    pub fn at(kind: DbKind, backslash_escapes: bool, offset: u64, line: u64) -> Splitter {
        Splitter {
            kind,
            backslash: backslash_escapes,
            delimiter: b";".to_vec(),
            cur: Vec::new(),
            start: None,
            start_line: 0,
            start_offset: 0,
            line,
            offset,
            state: State::Normal,
        }
    }

    /// The session's `sql_mode` changed (a statement of the file set it).
    pub fn set_backslash_escapes(&mut self, on: bool) {
        self.backslash = on;
    }

    /// Lines fed so far.
    pub fn line(&self) -> u64 {
        self.line
    }

    /// Byte offset of the next line.
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// A line of `len` bytes read past the splitter (the rows of a `COPY`), so line numbers
    /// and offsets stay right.
    pub fn skip_line(&mut self, len: u64) {
        self.line += 1;
        self.offset += len;
    }

    /// Mark the statement's start at byte `i` of the line (`seg`: where the part of the line
    /// not yet in `cur` begins).
    fn mark(&mut self, i: usize, seg: usize, line_offset: u64) {
        if self.start.is_none() {
            self.start = Some(self.cur.len() + i - seg);
            self.start_line = self.line;
            self.start_offset = line_offset + i as u64;
        }
    }

    fn emit(&mut self, out: &mut Vec<RawStatement>) {
        if let Some(s) = self.start.take() {
            let sql = trim_end(&self.cur[s..]).to_vec();
            if !sql.is_empty() {
                out.push(RawStatement {
                    sql,
                    line: self.start_line,
                    offset: self.start_offset,
                });
            }
        }
        self.cur.clear();
    }

    /// Feed one line (with its line break, if it has one); returns the statements it ended.
    pub fn push_line(&mut self, line: &str) -> Vec<Statement> {
        self.feed(line.as_bytes())
            .into_iter()
            .map(|r| Statement {
                sql: String::from_utf8_lossy(&r.sql).into_owned(),
                line: r.line,
            })
            .collect()
    }

    /// [`Splitter::push_line`] for a line of bytes.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<RawStatement> {
        self.line += 1;
        let line_offset = self.offset;
        self.offset += bytes.len() as u64;
        let mut out = Vec::new();
        let mysql = self.kind == DbKind::Mysql;
        if self.state == State::Normal && self.start.is_none() {
            let t = trim_start(bytes);
            if mysql
                && t.len() > 9
                && t[..9].eq_ignore_ascii_case(b"delimiter")
                && t[9].is_ascii_whitespace()
            {
                let d: Vec<u8> = trim_start(&t[9..])
                    .iter()
                    .take_while(|c| !c.is_ascii_whitespace())
                    .copied()
                    .collect();
                if !d.is_empty() {
                    self.delimiter = d;
                }
                self.cur.clear();
                return out;
            }
            // psql's own commands (`\connect`, `\restrict` in newer pg_dump files) are not
            // SQL: a line of their own, not sent to the server.
            if !mysql && t.first() == Some(&b'\\') {
                self.cur.clear();
                return out;
            }
        }
        let mut seg = 0;
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            match self.state.clone() {
                State::Normal => {
                    if bytes[i..].starts_with(&self.delimiter) {
                        self.cur.extend_from_slice(&bytes[seg..i]);
                        self.emit(&mut out);
                        i += self.delimiter.len();
                        seg = i;
                        continue;
                    }
                    let next = bytes.get(i + 1).copied();
                    match b {
                        b'\'' => {
                            let esc = if mysql {
                                self.backslash
                            } else {
                                i > 0
                                    && matches!(bytes[i - 1], b'E' | b'e')
                                    && (i < 2 || !ident_byte(bytes[i - 2]))
                            };
                            let at = if esc && !mysql { i - 1 } else { i };
                            self.mark(at, seg, line_offset);
                            self.state = State::Quote(b, esc);
                        }
                        b'"' => {
                            self.mark(i, seg, line_offset);
                            self.state = State::Quote(b, mysql && self.backslash);
                        }
                        b'`' if mysql => {
                            self.mark(i, seg, line_offset);
                            self.state = State::Quote(b, false);
                        }
                        b'-' if next == Some(b'-')
                            && (!mysql
                                || bytes.get(i + 2).is_none_or(|c| c.is_ascii_whitespace())) =>
                        {
                            // A comment to the end of the line.
                            break;
                        }
                        b'#' if mysql => break,
                        b'/' if next == Some(b'*') => {
                            // `/*! ... */` is code MySQL runs.
                            if mysql && bytes.get(i + 2) == Some(&b'!') {
                                self.mark(i, seg, line_offset);
                            }
                            self.state = State::Block(1);
                            i += 2;
                            continue;
                        }
                        b'$' if !mysql => {
                            self.mark(i, seg, line_offset);
                            if let Some(n) = dollar_tag(bytes, i) {
                                self.state = State::Dollar(bytes[i..i + n].to_vec());
                                i += n;
                                continue;
                            }
                        }
                        c if c.is_ascii_whitespace() => {}
                        _ => self.mark(i, seg, line_offset),
                    }
                    i += 1;
                }
                State::Quote(q, esc) => {
                    if esc && b == b'\\' {
                        i += 2;
                    } else if b == q {
                        if bytes.get(i + 1) == Some(&q) {
                            i += 2;
                        } else {
                            self.state = State::Normal;
                            i += 1;
                        }
                    } else {
                        i += 1;
                    }
                }
                State::Block(d) => {
                    if b == b'*' && bytes.get(i + 1) == Some(&b'/') {
                        self.state = if d == 1 {
                            State::Normal
                        } else {
                            State::Block(d - 1)
                        };
                        i += 2;
                    } else if !mysql && b == b'/' && bytes.get(i + 1) == Some(&b'*') {
                        self.state = State::Block(d + 1);
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                State::Dollar(tag) => {
                    if bytes[i..].starts_with(&tag) {
                        self.state = State::Normal;
                        i += tag.len();
                    } else {
                        i += 1;
                    }
                }
            }
        }
        // A line comment ends with the line; what came before it is kept.
        let end = i.min(bytes.len());
        self.cur.extend_from_slice(&bytes[seg..end.max(seg)]);
        if end < bytes.len() {
            self.cur.push(b'\n');
        }
        out
    }

    /// The statement left at the end of the file, if it did not end with a delimiter.
    pub fn finish(&mut self) -> Option<Statement> {
        self.finish_raw().map(|r| Statement {
            sql: String::from_utf8_lossy(&r.sql).into_owned(),
            line: r.line,
        })
    }

    /// [`Splitter::finish`] as bytes.
    pub fn finish_raw(&mut self) -> Option<RawStatement> {
        let mut out = Vec::new();
        self.emit(&mut out);
        self.state = State::Normal;
        out.pop()
    }
}

/// `COPY ... FROM STDIN`: the rows follow the statement in the file, up to a `\.` line.
pub fn is_copy_from_stdin(sql: &str) -> bool {
    let words: Vec<String> = sql
        .split_whitespace()
        .map(|w| w.to_ascii_uppercase())
        .collect();
    words.first().map(String::as_str) == Some("COPY")
        && words
            .windows(2)
            .any(|w| w[0] == "FROM" && w[1].trim_end_matches(';') == "STDIN")
}

/// Whether a statement may have changed MySQL's `sql_mode`.
pub fn sets_sql_mode(sql: &str) -> bool {
    sql.to_ascii_uppercase().contains("SQL_MODE")
}

/// A MySQL view's `CREATE` without its `DEFINER=...` clause: the account that imports the
/// dump becomes the definer (naming another one needs a privilege it may lack).
pub fn strip_definer(create: &str) -> String {
    if let Some(d) = create.find(" DEFINER=") {
        if let Some(s) = create[d..].find(" SQL SECURITY ") {
            return format!("{}{}", &create[..d], &create[d + s..]);
        }
    }
    create.to_string()
}

/// Views in an order that creates each after the views it reads: `defs` is (name, the
/// names it reads). A cycle (or a dependency outside the list) keeps name order.
pub fn order_views(mut defs: Vec<(String, Vec<String>)>) -> Vec<String> {
    defs.sort();
    let mut out = Vec::new();
    while !defs.is_empty() {
        let ready = defs.iter().position(|(n, deps)| {
            !deps
                .iter()
                .any(|d| d != n && defs.iter().any(|(o, _)| o == d))
        });
        let (n, _) = defs.remove(ready.unwrap_or(0));
        out.push(n);
    }
    out
}

/// The views a MySQL view definition reads: the other names in `views` it mentions in
/// backquotes (MySQL writes definitions fully quoted).
pub fn mysql_view_deps(definition: &str, name: &str, views: &[String]) -> Vec<String> {
    views
        .iter()
        .filter(|o| *o != name && definition.contains(&format!("`{}`", o.replace('`', "``"))))
        .cloned()
        .collect()
}

/// Whether a statement starts or ends a transaction (`BEGIN`, `START TRANSACTION`, `COMMIT`,
/// `END`; `rollback`: `ROLLBACK`, `ABORT`). `ROLLBACK TO` a savepoint is neither.
pub fn transaction_control(sql: &str) -> Option<bool> {
    let mut words = sql
        .split_whitespace()
        .map(|w| w.trim_end_matches(';').to_ascii_uppercase());
    let first = words.next()?;
    match first.as_str() {
        "BEGIN" | "COMMIT" | "END" => Some(false),
        "START" if words.next().as_deref() == Some("TRANSACTION") => Some(false),
        "ROLLBACK" | "ABORT" => {
            let rest: Vec<String> = words.take(2).collect();
            let word = |k: usize| rest.get(k).map(String::as_str);
            let to_savepoint = word(0) == Some("TO")
                || (matches!(word(0), Some("WORK" | "TRANSACTION")) && word(1) == Some("TO"));
            (!to_savepoint).then_some(true)
        }
        _ => None,
    }
}

/// Reads CSV records: fields split by `delimiter`, `"` quotes (doubled inside), line breaks
/// inside quotes kept. A UTF-8 byte order mark before the first record is skipped.
pub struct CsvReader<R> {
    r: R,
    delimiter: char,
    first: bool,
    /// Lines read so far.
    pub line: u64,
    /// Bytes read so far.
    pub bytes: u64,
}

impl<R: BufRead> CsvReader<R> {
    pub fn new(r: R, delimiter: char) -> CsvReader<R> {
        CsvReader {
            r,
            delimiter,
            first: true,
            line: 0,
            bytes: 0,
        }
    }

    /// The next record, `None` at the end. Blank lines are skipped.
    pub fn next_record(&mut self) -> std::io::Result<Option<Vec<String>>> {
        let mut fields = Vec::new();
        let mut field = String::new();
        let mut quoted = false;
        let mut in_quotes = false;
        let mut started = false;
        let mut line = String::new();
        loop {
            line.clear();
            let n = self.r.read_line(&mut line)?;
            self.bytes += n as u64;
            if n == 0 {
                if !started {
                    return Ok(None);
                }
                // The file ended inside quotes: it is cut short, not a record.
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "the file ends inside a quoted field",
                ));
            }
            self.line += 1;
            let mut text = line.as_str();
            if self.first {
                self.first = false;
                text = text.strip_prefix('\u{feff}').unwrap_or(text);
            }
            if !started && !in_quotes && text.trim_end_matches(['\r', '\n']).is_empty() {
                continue;
            }
            started = true;
            let mut chars = text.chars().peekable();
            while let Some(c) = chars.next() {
                if in_quotes {
                    if c == '"' {
                        if chars.peek() == Some(&'"') {
                            chars.next();
                            field.push('"');
                        } else {
                            in_quotes = false;
                        }
                    } else {
                        field.push(c);
                    }
                } else if c == '"' && !quoted && field.is_empty() {
                    in_quotes = true;
                    quoted = true;
                } else if c == self.delimiter {
                    fields.push(std::mem::take(&mut field));
                    quoted = false;
                } else if c == '\n' || (c == '\r' && chars.peek().is_none_or(|n| *n == '\n')) {
                    if c == '\r' {
                        chars.next();
                    }
                    fields.push(std::mem::take(&mut field));
                    return Ok(Some(fields));
                } else {
                    field.push(c);
                }
            }
            if !in_quotes {
                // The last line had no line break.
                fields.push(std::mem::take(&mut field));
                return Ok(Some(fields));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(kind: DbKind, text: &str) -> Vec<(String, u64)> {
        let mut s = Splitter::new(kind, true);
        let mut out = Vec::new();
        for l in text.split_inclusive('\n') {
            out.extend(s.push_line(l));
        }
        out.extend(s.finish());
        out.into_iter().map(|st| (st.sql, st.line)).collect()
    }

    #[test]
    fn literals() {
        let m = DbKind::Mysql;
        let p = DbKind::Postgres;
        assert_eq!(text_literal(m, "a'b\\c\nd\0", true), "'a''b\\\\c\\nd\\0'");
        assert_eq!(text_literal(m, "a'b\\c", false), "'a''b\\c'");
        assert_eq!(text_literal(p, "a'b\\c\n", true), "'a''b\\c\n'");
        assert_eq!(literal(m, &Value::Null, false), "NULL");
        assert_eq!(literal(m, &Value::Text("AB".into()), true), "X'4142'");
        assert_eq!(
            literal(m, &Value::Bytes { hex: "ff00".into() }, false),
            "X'ff00'"
        );
        assert_eq!(
            literal(p, &Value::Bytes { hex: "ff".into() }, false),
            "'\\xff'::bytea"
        );
        assert_eq!(literal(p, &Value::Text("\\x41".into()), false), "'\\x41'");
        assert!(mysql_binary_type("LONGBLOB") && !mysql_binary_type("text"));
    }

    #[test]
    fn batches_inserts() {
        let mut ins = Inserts::new("`t`", &["`a`".into(), "`b`".into()], false);
        assert_eq!(ins.push(&["1".into(), "'x'".into()]), None);
        assert_eq!(ins.push(&["2".into(), "NULL".into()]), None);
        assert_eq!(
            ins.finish().unwrap(),
            "INSERT INTO `t` (`a`, `b`) VALUES\n(1, 'x'),\n(2, NULL);\n"
        );
        assert_eq!(ins.finish(), None);
        let big = "x".repeat(Inserts::LIMIT);
        assert!(ins.push(&[big, "1".into()]).is_some());
        let mut o = Inserts::new("\"t\"", &["\"id\"".into()], true);
        o.push(&["1".into()]);
        assert!(o
            .finish()
            .unwrap()
            .starts_with("INSERT INTO \"t\" (\"id\") OVERRIDING SYSTEM VALUE VALUES"));
    }

    #[test]
    fn splits_mysql() {
        let sql = "-- a dump\nSET NAMES utf8mb4;\n/*!40101 SET @X=1 */;\n\
                   INSERT INTO t VALUES ('a;b', 'it''s', 'x\\';y'), (\"q;\", `c;`);\n\
                   # comment; here\nSELECT 1; SELECT 2;\n\
                   DELIMITER ;;\nCREATE TRIGGER tr BEFORE INSERT ON t FOR EACH ROW BEGIN SET @a = 1; END;;\nDELIMITER ;\n\
                   SELECT '--not a comment'--x\n;\nSELECT 3";
        let got = split(DbKind::Mysql, sql);
        let texts: Vec<&str> = got.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "SET NAMES utf8mb4",
                "/*!40101 SET @X=1 */",
                "INSERT INTO t VALUES ('a;b', 'it''s', 'x\\';y'), (\"q;\", `c;`)",
                "SELECT 1",
                "SELECT 2",
                "CREATE TRIGGER tr BEFORE INSERT ON t FOR EACH ROW BEGIN SET @a = 1; END",
                "SELECT '--not a comment'--x",
                "SELECT 3",
            ]
        );
        assert_eq!(got[0].1, 2);
        assert_eq!(got[3].1, 6);
        // Without backslash escapes a backslash ends nothing.
        let mut s = Splitter::new(DbKind::Mysql, false);
        let out = s.push_line("SELECT 'a\\'; SELECT 2;\n");
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn splits_postgres() {
        let sql = "--\n-- header\n--\nSET standard_conforming_strings = on;\n\
                   CREATE FUNCTION f() RETURNS int AS $body$ SELECT 1; $body$ LANGUAGE sql;\n\
                   SELECT $$a;b$$, E'x\\';y', 'c\\', $1;\n\
                   /* outer /* inner; */ still; */ SELECT 4;\n\
                   SELECT 'multi\nline;';\n";
        let got = split(DbKind::Postgres, sql);
        let texts: Vec<&str> = got.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "SET standard_conforming_strings = on",
                "CREATE FUNCTION f() RETURNS int AS $body$ SELECT 1; $body$ LANGUAGE sql",
                "SELECT $$a;b$$, E'x\\';y', 'c\\', $1",
                "SELECT 4",
                "SELECT 'multi\nline;'",
            ]
        );
        assert_eq!(got[0].1, 4);
        assert_eq!(got[4].1, 8);
        assert!(is_copy_from_stdin("COPY public.t (a, b) FROM stdin"));
        // psql commands are skipped; statements know where they start.
        let mut sp = Splitter::new(DbKind::Postgres, true);
        assert!(sp.feed(b"\\restrict abc\n").is_empty());
        let got = sp.feed(b"SELECT 1;  SELECT\n");
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].offset, got[0].line), (14, 2));
        let got = sp.feed(b" 2;\n");
        assert_eq!(got[0].sql, b"SELECT\n 2");
        assert_eq!((got[0].offset, got[0].line), (25, 2));
        // Bytes that are not UTF-8 pass through.
        let mut sp = Splitter::new(DbKind::Mysql, true);
        let got = sp.feed(b"INSERT INTO t VALUES ('\xff\\'');\n");
        assert_eq!(got[0].sql, b"INSERT INTO t VALUES ('\xff\\'')");
        assert!(!is_copy_from_stdin("COPY t TO STDOUT"));
        assert!(sets_sql_mode("SET @OLD=@@SQL_MODE, sql_mode='X'"));
    }

    #[test]
    fn views_and_definers() {
        assert_eq!(
            strip_definer("CREATE ALGORITHM=UNDEFINED DEFINER=`root`@`%` SQL SECURITY DEFINER VIEW `v` AS select 1"),
            "CREATE ALGORITHM=UNDEFINED SQL SECURITY DEFINER VIEW `v` AS select 1"
        );
        assert_eq!(
            strip_definer("CREATE VIEW v AS select 1"),
            "CREATE VIEW v AS select 1"
        );
        let views: Vec<String> = vec!["a".into(), "b".into(), "c".into()];
        let defs = [
            ("a", "select * from `s`.`b`"),
            ("b", "select * from `s`.`c`"),
            ("c", "select * from `s`.`t`"),
        ];
        let order = order_views(
            defs.iter()
                .map(|(n, d)| (n.to_string(), mysql_view_deps(d, n, &views)))
                .collect(),
        );
        assert_eq!(order, vec!["c", "b", "a"]);
        // PostgreSQL: dependencies from the catalog, whatever the creation order.
        let order = order_views(vec![
            ("old".into(), vec!["new".into(), "some_table".into()]),
            ("new".into(), vec![]),
        ]);
        assert_eq!(order, vec!["new", "old"]);
        assert_eq!(transaction_control("BEGIN"), Some(false));
        assert_eq!(
            transaction_control("start transaction isolation level serializable"),
            Some(false)
        );
        assert_eq!(transaction_control("COMMIT"), Some(false));
        assert_eq!(transaction_control("end"), Some(false));
        assert_eq!(transaction_control("ROLLBACK"), Some(true));
        assert_eq!(transaction_control("ROLLBACK TO SAVEPOINT a"), None);
        assert_eq!(transaction_control("rollback work to a"), None);
        assert_eq!(transaction_control("SELECT 1"), None);
        assert_eq!(transaction_control("START REPLICA"), None);
    }

    #[test]
    fn reads_csv() {
        let data = "\u{feff}id;name;note\r\n1;\"Rossi; Mario\";\"say \"\"hi\"\"\"\n\n2;Bianchi;\"two\nlines\"\n3;;";
        let mut r = CsvReader::new(data.as_bytes(), ';');
        assert_eq!(
            r.next_record().unwrap().unwrap(),
            vec!["id", "name", "note"]
        );
        assert_eq!(
            r.next_record().unwrap().unwrap(),
            vec!["1", "Rossi; Mario", "say \"hi\""]
        );
        assert_eq!(
            r.next_record().unwrap().unwrap(),
            vec!["2", "Bianchi", "two\nlines"]
        );
        assert_eq!(r.next_record().unwrap().unwrap(), vec!["3", "", ""]);
        assert_eq!(r.next_record().unwrap(), None);
        assert_eq!(r.line, 6);
        assert_eq!(r.bytes, data.len() as u64);
        // Cut short inside quotes: an error, not a record.
        let mut r = CsvReader::new("id;name\n1;\"trunc".as_bytes(), ';');
        r.next_record().unwrap();
        assert_eq!(
            r.next_record().unwrap_err().kind(),
            std::io::ErrorKind::UnexpectedEof
        );
        // A DELIMITER check must not cut a multi-byte character.
        let mut sp = Splitter::new(DbKind::Mysql, true);
        assert!(sp.push_line("-- 12345\u{e9} comment\n").is_empty());
        assert_eq!(sp.push_line("SELECT 1;\n").len(), 1);
    }
}
