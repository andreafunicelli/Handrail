//! An SQL backup read without a server: one pass over the file builds an index of its
//! tables (where each one's rows are, how many), then pages of rows are read back from
//! there, and a table can be saved as CSV, JSON or SQL. Plain SQL as `mysqldump`, `pg_dump`
//! (plain format, INSERTs or COPY) and our own export write it; the file can be any size.

use crate::dump::{self, RawStatement, Splitter};
use crate::{DbKind, Error, Result, Value};
use serde::Serialize;
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};

/// Where a run of a table's rows is in the file.
#[derive(Clone, Debug)]
struct Chunk {
    /// Byte offset of the first statement (INSERT or COPY).
    offset: u64,
    /// Its line, 1-based.
    line: u64,
    /// COPY: where the rows start (the line after the statement).
    data: Option<u64>,
    rows: u64,
    /// The column list the statements name, if they name one.
    columns: Option<Vec<String>>,
}

/// A table (or view) of the backup.
#[derive(Clone, Debug, Serialize)]
pub struct BackupTable {
    pub schema: Option<String>,
    pub name: String,
    pub view: bool,
    /// From its CREATE TABLE, else from the first INSERT or COPY that names them.
    pub columns: Vec<String>,
    pub rows: u64,
    /// The statement that creates it, as the file has it.
    pub create: Option<String>,
    #[serde(skip)]
    chunks: Vec<Chunk>,
    /// DROP before the CREATE, if the file has one.
    #[serde(skip)]
    drop: Option<String>,
    /// What creates its sequences (before the table).
    #[serde(skip)]
    before: Vec<String>,
    /// Indexes, constraints, defaults, sequence positions (after the rows).
    #[serde(skip)]
    after: Vec<String>,
}

/// What the file holds.
#[derive(Clone, Debug, Serialize)]
pub struct BackupIndex {
    pub kind: DbKind,
    pub tables: Vec<BackupTable>,
    pub statements: u64,
    pub bytes: u64,
    /// Session settings the file starts with (SET NAMES, sql_mode...), kept for a table
    /// saved as SQL.
    #[serde(skip)]
    preamble: Vec<String>,
}

/// Reads the backup again from a byte offset (a plain file seeks; a compressed one is
/// read from its start up to there).
pub trait Source {
    fn open_at(&mut self, offset: u64) -> io::Result<Box<dyn BufRead + '_>>;
}

/// A file's dialect from its first bytes.
pub fn detect(head: &[u8]) -> DbKind {
    let h = String::from_utf8_lossy(&head[..head.len().min(64 * 1024)]).to_ascii_lowercase();
    let pg = [
        "postgresql database dump",
        "pg_dump",
        "standard_conforming_strings",
        "\ncopy ",
        "-- schema:",
        "search_path",
    ];
    let my = [
        "mysql dump",
        "mysqldump",
        "mariadb dump",
        "engine=",
        "-- database:",
        "foreign_key_checks",
        "/*!40",
    ];
    let score = |words: &[&str]| words.iter().filter(|w| h.contains(*w)).count();
    if score(&pg) > score(&my) || (score(&my) == 0 && !h.contains('`') && h.contains('"')) {
        DbKind::Postgres
    } else {
        DbKind::Mysql
    }
}

// ---------- a cursor over a statement ----------

struct Cur<'a> {
    b: &'a [u8],
    i: usize,
    kind: DbKind,
}

fn word_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c >= 0x80
}

impl<'a> Cur<'a> {
    fn new(b: &'a [u8], kind: DbKind) -> Cur<'a> {
        Cur { b, i: 0, kind }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    /// Skip blanks and comments.
    fn ws(&mut self) {
        loop {
            while self.peek().is_some_and(|c| c.is_ascii_whitespace()) {
                self.i += 1;
            }
            let rest = &self.b[self.i..];
            if rest.starts_with(b"--") || (self.kind == DbKind::Mysql && rest.starts_with(b"#")) {
                while self.peek().is_some_and(|c| c != b'\n') {
                    self.i += 1;
                }
            } else if rest.starts_with(b"/*") && !rest.starts_with(b"/*!") {
                match rest[2..].windows(2).position(|w| w == b"*/") {
                    Some(p) => self.i += p + 4,
                    None => self.i = self.b.len(),
                }
            } else {
                return;
            }
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        self.ws();
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    /// A bare word, as written.
    fn word(&mut self) -> Option<String> {
        self.ws();
        let s = self.i;
        while self.peek().is_some_and(word_byte) {
            self.i += 1;
        }
        (self.i > s).then(|| String::from_utf8_lossy(&self.b[s..self.i]).into_owned())
    }

    /// The keyword `k` (any case), consumed only if it is there.
    fn kw(&mut self, k: &str) -> bool {
        self.ws();
        let end = self.i + k.len();
        if end <= self.b.len()
            && self.b[self.i..end].eq_ignore_ascii_case(k.as_bytes())
            && !self.b.get(end).is_some_and(|c| word_byte(*c))
        {
            self.i = end;
            true
        } else {
            false
        }
    }

    /// A name part: quoted (`` ` `` or `"`) or bare.
    fn ident(&mut self) -> Option<String> {
        self.ws();
        match self.peek()? {
            q @ (b'`' | b'"') => {
                self.i += 1;
                let mut out = Vec::new();
                while let Some(c) = self.peek() {
                    self.i += 1;
                    if c == q {
                        if self.peek() == Some(q) {
                            self.i += 1;
                        } else {
                            return Some(String::from_utf8_lossy(&out).into_owned());
                        }
                    }
                    out.push(c);
                }
                None
            }
            _ => self.word(),
        }
    }

    /// `[schema.]name`.
    fn name(&mut self) -> Option<(Option<String>, String)> {
        let first = self.ident()?;
        if self.peek() == Some(b'.') {
            self.i += 1;
            let second = self.ident()?;
            return Some((Some(first), second));
        }
        Some((None, first))
    }

    /// `(a, b, c)`, if a list of names is next.
    fn names(&mut self) -> Option<Vec<String>> {
        let save = self.i;
        if !self.eat(b'(') {
            return None;
        }
        let mut out = Vec::new();
        loop {
            match self.ident() {
                Some(n) => out.push(n),
                None => {
                    self.i = save;
                    return None;
                }
            }
            if self.eat(b',') {
                continue;
            }
            if self.eat(b')') {
                return Some(out);
            }
            self.i = save;
            return None;
        }
    }

    /// Skip to the next `,` or `)` at this depth (not consuming it), past quotes.
    fn skip_item(&mut self) {
        let mut depth = 0usize;
        while let Some(c) = self.peek() {
            match c {
                b'\'' | b'"' | b'`' => {
                    self.string_bytes(c, self.kind == DbKind::Mysql && c != b'`');
                    continue;
                }
                b'(' => depth += 1,
                b')' if depth == 0 => return,
                b')' => depth -= 1,
                b',' if depth == 0 => return,
                _ => {}
            }
            self.i += 1;
        }
    }

    /// A quoted string starting at the cursor; its content unescaped.
    fn string_bytes(&mut self, q: u8, escapes: bool) -> Vec<u8> {
        self.i += 1;
        let mut out = Vec::new();
        while let Some(c) = self.peek() {
            self.i += 1;
            if c == q {
                if self.peek() == Some(q) {
                    self.i += 1;
                    out.push(q);
                    continue;
                }
                return out;
            }
            if c == b'\\' && escapes {
                let Some(e) = self.peek() else { break };
                self.i += 1;
                if self.kind == DbKind::Mysql {
                    match e {
                        b'0' => out.push(0),
                        b'b' => out.push(8),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'Z' => out.push(0x1a),
                        // Kept with their backslash (LIKE patterns).
                        b'%' | b'_' => out.extend_from_slice(&[b'\\', e]),
                        other => out.push(other),
                    }
                } else {
                    self.pg_escape(e, &mut out);
                }
                continue;
            }
            out.push(c);
        }
        out
    }

    /// One escape of a PostgreSQL `E'...'` string (the byte after the backslash is `e`).
    fn pg_escape(&mut self, e: u8, out: &mut Vec<u8>) {
        let hex_digits = |cur: &mut Cur, max: usize| {
            let s = cur.i;
            while cur.i - s < max && cur.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
                cur.i += 1;
            }
            u32::from_str_radix(std::str::from_utf8(&cur.b[s..cur.i]).unwrap_or("0"), 16)
                .unwrap_or(0)
        };
        match e {
            b'b' => out.push(8),
            b'f' => out.push(12),
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'x' => out.push(hex_digits(self, 2) as u8),
            b'u' | b'U' => {
                let n = hex_digits(self, if e == b'u' { 4 } else { 8 });
                let mut buf = [0; 4];
                out.extend_from_slice(
                    char::from_u32(n)
                        .unwrap_or('\u{fffd}')
                        .encode_utf8(&mut buf)
                        .as_bytes(),
                );
            }
            b'0'..=b'7' => {
                let mut n = u32::from(e - b'0');
                for _ in 0..2 {
                    match self.peek() {
                        Some(d @ b'0'..=b'7') => {
                            n = n * 8 + u32::from(d - b'0');
                            self.i += 1;
                        }
                        _ => break,
                    }
                }
                out.push(n as u8);
            }
            other => out.push(other),
        }
    }

    /// Hex digits as bytes (`X'..'`, `0x..`, `\x..` of bytea).
    fn hex(digits: &[u8]) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(digits.len() / 2);
        let mut i = 0;
        while i + 1 < digits.len() {
            out.push(u8::from_str_radix(std::str::from_utf8(&digits[i..i + 2]).ok()?, 16).ok()?);
            i += 2;
        }
        (i == digits.len()).then_some(out)
    }

    /// Skip a PostgreSQL `::type` after a value; returns the type's name (lower case).
    fn cast(&mut self) -> Option<String> {
        self.ws();
        if !self.b[self.i..].starts_with(b"::") {
            return None;
        }
        self.i += 2;
        let s = self.i;
        self.skip_item();
        Some(
            String::from_utf8_lossy(&self.b[s..self.i])
                .trim()
                .to_ascii_lowercase(),
        )
    }

    /// One value of a VALUES tuple. What is not a literal (a function call, an expression)
    /// is kept as its text.
    fn value(&mut self) -> Value {
        self.ws();
        let start = self.i;
        let Some(c) = self.peek() else {
            return Value::Null;
        };
        let mysql = self.kind == DbKind::Mysql;
        let rest = &self.b[self.i..];
        let after_quote = |n: usize| rest.get(n) == Some(&b'\'');
        // Hex and bit literals.
        if matches!(c, b'x' | b'X') && after_quote(1) {
            self.i += 1;
            let raw = self.string_bytes(b'\'', false);
            return Self::hex(&raw).map_or(Value::Null, |b| Value::from_bytes(&b));
        }
        if c == b'0' && matches!(rest.get(1), Some(b'x' | b'X')) {
            self.i += 2;
            let s = self.i;
            while self.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
                self.i += 1;
            }
            let digits = &self.b[s..self.i];
            return Self::hex(digits).map_or_else(
                || Value::Text(String::from_utf8_lossy(digits).into_owned()),
                |b| Value::from_bytes(&b),
            );
        }
        if matches!(c, b'b' | b'B') && after_quote(1) {
            self.i += 1;
            let bits = self.string_bytes(b'\'', false);
            return Value::Text(format!("b'{}'", String::from_utf8_lossy(&bits)));
        }
        if !mysql && matches!(c, b'e' | b'E') && after_quote(1) {
            self.i += 1;
            let s = self.string_bytes(b'\'', true);
            self.cast();
            return Value::from_bytes(&s);
        }
        if matches!(c, b'n' | b'N') && after_quote(1) {
            self.i += 1;
        }
        // MySQL charset introducers: _utf8mb4'..', _binary'..'.
        if mysql && c == b'_' {
            let save = self.i;
            self.i += 1;
            while self.peek().is_some_and(|c| c.is_ascii_alphanumeric()) {
                self.i += 1;
            }
            self.ws();
            if self.peek() != Some(b'\'') {
                self.i = save;
            }
        }
        match self.peek() {
            Some(b'\'') => {
                let s = self.string_bytes(b'\'', mysql);
                if let Some(ty) = self.cast() {
                    if ty == "bytea" && s.starts_with(b"\\x") {
                        if let Some(b) = Self::hex(&s[2..]) {
                            return Value::from_bytes(&b);
                        }
                    }
                }
                return Value::from_bytes(&s);
            }
            Some(c) if c.is_ascii_digit() || c == b'-' || c == b'+' || c == b'.' => {
                let s = self.i;
                self.i += 1;
                while self.peek().is_some_and(|c| {
                    c.is_ascii_alphanumeric() || c == b'.' || c == b'+' || c == b'-'
                }) {
                    let prev = self.b[self.i - 1];
                    if matches!(self.b[self.i], b'+' | b'-') && !matches!(prev, b'e' | b'E') {
                        break;
                    }
                    self.i += 1;
                }
                let text = String::from_utf8_lossy(&self.b[s..self.i]).into_owned();
                self.cast();
                self.ws();
                if matches!(self.peek(), Some(b',' | b')') | None) {
                    return Value::Text(text);
                }
            }
            _ => {
                if self.kw("NULL") {
                    self.cast();
                    self.ws();
                    if matches!(self.peek(), Some(b',' | b')') | None) {
                        return Value::Null;
                    }
                }
            }
        }
        // Anything else: its text.
        self.i = start;
        self.skip_item();
        Value::Text(
            String::from_utf8_lossy(&self.b[start..self.i])
                .trim()
                .to_string(),
        )
    }

    /// The rows of `VALUES (..), (..)`; `None` when the text is not a list of tuples.
    fn tuples(&mut self, mut each: impl FnMut(Vec<Value>) -> bool) -> Option<u64> {
        let mut n = 0;
        loop {
            if !self.eat(b'(') {
                return (n > 0).then_some(n);
            }
            let mut row = Vec::new();
            if !self.eat(b')') {
                loop {
                    row.push(self.value());
                    if self.eat(b',') {
                        continue;
                    }
                    if self.eat(b')') {
                        break;
                    }
                    return None;
                }
            }
            n += 1;
            if !each(row) {
                return Some(n);
            }
            if !self.eat(b',') {
                return Some(n);
            }
        }
    }
}

/// `/*!40101 SET ... */` as MySQL runs it: the markers out, the code kept.
fn unwrap_conditional(sql: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(sql.len());
    let mut i = 0;
    let mut quote: Option<u8> = None;
    while i < sql.len() {
        let c = sql[i];
        if let Some(q) = quote {
            out.push(c);
            if c == b'\\' && q != b'`' {
                if let Some(n) = sql.get(i + 1) {
                    out.push(*n);
                }
                i += 2;
                continue;
            }
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if sql[i..].starts_with(b"/*!") {
            i += 3;
            while sql.get(i).is_some_and(|c| c.is_ascii_digit()) {
                i += 1;
            }
            continue;
        }
        if sql[i..].starts_with(b"*/") {
            i += 2;
            continue;
        }
        if matches!(c, b'\'' | b'"' | b'`') {
            quote = Some(c);
        }
        out.push(c);
        i += 1;
    }
    out
}

/// `[schema.]name`.
type Name = (Option<String>, String);
/// A data statement's table and the columns it names.
type Head = Option<(Name, Option<Vec<String>>)>;
/// Called with each row: its position, column names and values; false stops.
type RowFn<'a> = dyn FnMut(u64, &[String], Vec<Value>) -> bool + 'a;

/// The INSERT's target, column list and where its VALUES start; `None` for any other
/// statement (or an INSERT ... SELECT).
fn insert_head(c: &mut Cur) -> Head {
    if !(c.kw("INSERT") || c.kw("REPLACE")) {
        return None;
    }
    for k in ["LOW_PRIORITY", "DELAYED", "HIGH_PRIORITY", "IGNORE"] {
        c.kw(k);
    }
    c.kw("INTO");
    let name = c.name()?;
    let cols = c.names();
    if c.kw("OVERRIDING") {
        c.word();
        c.kw("VALUE");
    }
    if !(c.kw("VALUES") || c.kw("VALUE")) {
        return None;
    }
    Some((name, cols))
}

/// The COPY's table and column list, if it reads its rows from the file.
fn copy_head(c: &mut Cur, sql: &str) -> Head {
    if !c.kw("COPY") || !dump::is_copy_from_stdin(sql) {
        return None;
    }
    let name = c.name()?;
    Some((name, c.names()))
}

/// The column names of a CREATE TABLE body (the cursor at its `(`).
fn create_columns(c: &mut Cur) -> Vec<String> {
    const NOT_COLUMNS: &[&str] = &[
        "PRIMARY",
        "KEY",
        "INDEX",
        "UNIQUE",
        "CONSTRAINT",
        "FOREIGN",
        "CHECK",
        "FULLTEXT",
        "SPATIAL",
        "EXCLUDE",
        "LIKE",
        "PERIOD",
    ];
    let mut out = Vec::new();
    if !c.eat(b'(') {
        return out;
    }
    loop {
        c.ws();
        let quoted = matches!(c.peek(), Some(b'`' | b'"'));
        if let Some(n) = c.ident() {
            if quoted || !NOT_COLUMNS.iter().any(|k| k.eq_ignore_ascii_case(&n)) {
                out.push(n);
            }
        }
        c.skip_item();
        if c.eat(b',') {
            continue;
        }
        return out;
    }
}

/// Builds the index as statements arrive.
struct Builder {
    kind: DbKind,
    tables: Vec<BackupTable>,
    by_name: HashMap<(String, String), usize>,
    /// MySQL: the database a `USE` picked.
    current_db: Option<String>,
    drops: HashMap<(String, String), String>,
    /// Sequences: their statements, and the table that owns each.
    seq_stmts: HashMap<String, (Vec<String>, Vec<String>)>,
    seq_owner: HashMap<String, (Option<String>, String)>,
    preamble: Vec<String>,
    seen_table: bool,
    statements: u64,
    /// The table the last statement added rows to: the next one may extend its chunk.
    last_chunk: Option<usize>,
}

/// A name as a lookup key: schema (or ""), table.
fn key(schema: &Option<String>, name: &str) -> (String, String) {
    (schema.clone().unwrap_or_default(), name.to_string())
}

impl Builder {
    fn new(kind: DbKind) -> Builder {
        Builder {
            kind,
            tables: Vec::new(),
            by_name: HashMap::new(),
            current_db: None,
            drops: HashMap::new(),
            seq_stmts: HashMap::new(),
            seq_owner: HashMap::new(),
            preamble: Vec::new(),
            seen_table: false,
            statements: 0,
            last_chunk: None,
        }
    }

    fn qualify(&self, (schema, name): (Option<String>, String)) -> (Option<String>, String) {
        (schema.or_else(|| self.current_db.clone()), name)
    }

    /// A sequence's key: `schema.name` (two schemas may each have `orders_id_seq`); an
    /// unqualified name matches the one sequence of that name seen so far, if only one.
    fn seq_key(&self, n: &(Option<String>, String)) -> String {
        let n = self.qualify(n.clone());
        match &n.0 {
            Some(s) => format!("{s}.{}", n.1),
            None => {
                let suffix = format!(".{}", n.1);
                let same: Vec<&String> = self
                    .seq_stmts
                    .keys()
                    .filter(|k| k.ends_with(&suffix) || **k == n.1)
                    .collect();
                if same.len() == 1 {
                    same[0].clone()
                } else {
                    n.1
                }
            }
        }
    }

    fn table(&mut self, n: (Option<String>, String)) -> usize {
        let n = self.qualify(n);
        let k = key(&n.0, &n.1);
        if let Some(i) = self.by_name.get(&k) {
            return *i;
        }
        // An unqualified name matches a single table of that name.
        if n.0.is_none() {
            let same: Vec<usize> = self
                .tables
                .iter()
                .enumerate()
                .filter(|(_, t)| t.name == n.1)
                .map(|(i, _)| i)
                .collect();
            if same.len() == 1 {
                return same[0];
            }
        }
        self.tables.push(BackupTable {
            schema: n.0,
            name: n.1,
            view: false,
            columns: Vec::new(),
            rows: 0,
            create: None,
            chunks: Vec::new(),
            drop: None,
            before: Vec::new(),
            after: Vec::new(),
        });
        self.by_name.insert(k, self.tables.len() - 1);
        self.tables.len() - 1
    }

    fn add_rows(
        &mut self,
        t: usize,
        st: &RawStatement,
        data: Option<u64>,
        rows: u64,
        columns: Option<Vec<String>>,
    ) {
        let table = &mut self.tables[t];
        if table.columns.is_empty() {
            if let Some(c) = &columns {
                table.columns = c.clone();
            }
        }
        table.rows += rows;
        // Consecutive INSERTs into the same table share an entry (up to 10 000 rows, so a
        // page far into the table does not reread it all).
        if data.is_none() && self.last_chunk == Some(t) {
            if let Some(last) = table.chunks.last_mut() {
                if last.data.is_none() && last.columns == columns && last.rows < 10_000 {
                    last.rows += rows;
                    return;
                }
            }
        }
        table.chunks.push(Chunk {
            offset: st.offset,
            line: st.line,
            data,
            rows,
            columns,
        });
        self.last_chunk = Some(t);
    }

    /// One statement; for a COPY from the file, the table its rows go to.
    fn statement(&mut self, st: &RawStatement) -> Option<(usize, Option<Vec<String>>)> {
        self.statements += 1;
        let text = if self.kind == DbKind::Mysql && st.sql.starts_with(b"/*!") {
            unwrap_conditional(&st.sql)
        } else {
            st.sql.clone()
        };
        let raw = String::from_utf8_lossy(&st.sql).into_owned();
        let mut c = Cur::new(&text, self.kind);
        let first = c.word().unwrap_or_default().to_ascii_uppercase();
        c.i = 0;
        let mut last = None;
        match first.as_str() {
            "INSERT" | "REPLACE" => {
                if let Some((name, cols)) = insert_head(&mut c) {
                    let rows = c.tuples(|_| true).unwrap_or(0);
                    let t = self.table(name);
                    self.add_rows(t, st, None, rows, cols);
                    self.seen_table = true;
                    return None;
                }
            }
            "COPY" => {
                let s = String::from_utf8_lossy(&text).into_owned();
                if let Some((name, cols)) = copy_head(&mut c, &s) {
                    self.seen_table = true;
                    self.last_chunk = None;
                    return Some((self.table(name), cols));
                }
            }
            "USE" if self.kind == DbKind::Mysql => {
                c.word();
                self.current_db = c.ident();
            }
            "CREATE" => {
                c.word();
                last = self.create(&mut c, raw.clone());
            }
            "DROP" => {
                c.word();
                let view = c.kw("VIEW") || (c.kw("MATERIALIZED") && c.kw("VIEW"));
                if view || c.kw("TABLE") {
                    c.kw("IF");
                    c.kw("EXISTS");
                    if let Some(n) = c.name() {
                        let n = self.qualify(n);
                        self.drops.insert(key(&n.0, &n.1), raw.clone());
                    }
                }
            }
            "ALTER" => {
                c.word();
                if c.kw("TABLE") {
                    c.kw("IF");
                    c.kw("EXISTS");
                    c.kw("ONLY");
                    if let Some(n) = c.name() {
                        // Owners name roles the target server may not have.
                        let upper = raw.to_ascii_uppercase();
                        if !upper.contains(" OWNER TO ")
                            && !upper.contains("DISABLE KEYS")
                            && !upper.contains("ENABLE KEYS")
                        {
                            let t = self.table(n);
                            self.tables[t].after.push(raw.clone());
                            last = Some(t);
                        }
                    }
                } else if c.kw("SEQUENCE") {
                    c.kw("IF");
                    c.kw("EXISTS");
                    if let Some(n) = c.name() {
                        let seq = self.seq_key(&n);
                        let upper = raw.to_ascii_uppercase();
                        if let Some(p) = find_ci(&text, b"OWNED BY ") {
                            let mut oc = Cur::new(&text[p + 9..], self.kind);
                            let parts = dotted(&mut oc);
                            if parts.len() >= 2 {
                                let table = parts[parts.len() - 2].clone();
                                let schema =
                                    (parts.len() >= 3).then(|| parts[parts.len() - 3].clone());
                                self.seq_owner.insert(seq.clone(), (schema, table));
                            }
                            self.seq_stmts.entry(seq).or_default().1.push(raw.clone());
                        } else if !upper.contains(" OWNER TO ") {
                            self.seq_stmts.entry(seq).or_default().0.push(raw.clone());
                        }
                    }
                }
            }
            "SELECT" => {
                // pg_catalog.setval('seq', n, true)
                if let Some(p) = find_ci(&text, b"setval(") {
                    let mut sc = Cur::new(&text[p + 7..], self.kind);
                    sc.ws();
                    if sc.peek() == Some(b'\'') {
                        let s = sc.string_bytes(b'\'', false);
                        let mut nc = Cur::new(&s, self.kind);
                        let parts = dotted(&mut nc);
                        if let Some(name) = parts.last() {
                            let schema = (parts.len() >= 2).then(|| parts[parts.len() - 2].clone());
                            let k = self.seq_key(&(schema, name.clone()));
                            self.seq_stmts.entry(k).or_default().1.push(raw.clone());
                        }
                    }
                }
            }
            _ => {}
        }
        if last.is_none() && !self.seen_table && self.preamble.len() < 100 {
            let upper = first.as_str();
            if matches!(upper, "SET") || st.sql.starts_with(b"/*!") {
                self.preamble.push(raw);
            }
        }
        self.last_chunk = None;
        None
    }

    /// `CREATE ...` after the word CREATE; the table it belongs to, if any.
    fn create(&mut self, c: &mut Cur, raw: String) -> Option<usize> {
        if c.kw("OR") {
            c.kw("REPLACE");
        }
        // Modifiers before TABLE / VIEW.
        loop {
            if c.kw("TEMPORARY") || c.kw("TEMP") {
                return None;
            }
            if c.kw("UNLOGGED") || c.kw("GLOBAL") || c.kw("LOCAL") {
                continue;
            }
            if c.kw("ALGORITHM") || c.kw("SQL") {
                c.eat(b'=');
                c.kw("SECURITY");
                c.word();
                continue;
            }
            if c.kw("DEFINER") {
                c.eat(b'=');
                c.ident();
                if c.eat(b'@') {
                    c.ident();
                }
                continue;
            }
            break;
        }
        let materialized = c.kw("MATERIALIZED");
        if c.kw("VIEW") {
            c.kw("IF");
            c.kw("NOT");
            c.kw("EXISTS");
            let n = c.name()?;
            let t = self.table(n);
            self.tables[t].view = true;
            self.tables[t].create = Some(raw);
            self.seen_table = true;
            return Some(t);
        }
        if materialized {
            return None;
        }
        if c.kw("TABLE") {
            c.kw("IF");
            c.kw("NOT");
            c.kw("EXISTS");
            let n = c.name()?;
            let t = self.table(n);
            let cols = create_columns(c);
            let table = &mut self.tables[t];
            table.view = false;
            if !cols.is_empty() {
                table.columns = cols;
            }
            table.create = Some(raw);
            self.seen_table = true;
            return Some(t);
        }
        if c.kw("SEQUENCE") {
            c.kw("IF");
            c.kw("NOT");
            c.kw("EXISTS");
            let n = c.name()?;
            let k = self.seq_key(&n);
            self.seq_stmts.entry(k).or_default().0.push(raw);
            return None;
        }
        // CREATE [UNIQUE] INDEX [CONCURRENTLY] [IF NOT EXISTS] [name] ON [ONLY] table
        c.kw("UNIQUE");
        if c.kw("INDEX") {
            while c.i < c.b.len() && !c.kw("ON") {
                c.ws();
                if matches!(c.peek(), Some(b'`' | b'"')) {
                    c.ident();
                } else if c.word().is_none() {
                    c.i += 1;
                }
            }
            c.kw("ONLY");
            let n = c.name()?;
            let t = self.table(n);
            self.tables[t].after.push(raw);
            return Some(t);
        }
        None
    }

    fn finish(mut self, bytes: u64) -> BackupIndex {
        for (k, d) in std::mem::take(&mut self.drops) {
            if let Some(i) = self.by_name.get(&k) {
                self.tables[*i].drop = Some(d);
            }
        }
        for (seq, (before, after)) in std::mem::take(&mut self.seq_stmts) {
            let Some(owner) = self.seq_owner.get(&seq).cloned() else {
                continue;
            };
            let k = key(&owner.0, &owner.1);
            let t = self.by_name.get(&k).copied().or_else(|| {
                let same: Vec<usize> = self
                    .tables
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| t.name == owner.1)
                    .map(|(i, _)| i)
                    .collect();
                (same.len() == 1).then(|| same[0])
            });
            if let Some(t) = t {
                self.tables[t].before.extend(before);
                self.tables[t].after.extend(after);
            }
        }
        BackupIndex {
            kind: self.kind,
            tables: self.tables,
            statements: self.statements,
            bytes,
            preamble: self.preamble,
        }
    }
}

/// Where `needle` (ASCII) first appears in `hay`, any case.
fn find_ci(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len())
        .position(|w| w.eq_ignore_ascii_case(needle))
}

/// `a.b.c` as its parts (quoted or not).
fn dotted(c: &mut Cur) -> Vec<String> {
    let mut parts = Vec::new();
    while let Some(p) = c.ident() {
        parts.push(p);
        if c.peek() != Some(b'.') {
            break;
        }
        c.i += 1;
    }
    parts
}

fn cancelled() -> Error {
    Error::Invalid("cancelled".into())
}

/// Read the backup once and index its tables. `report` gets the bytes read so far.
pub fn index(
    input: &mut dyn BufRead,
    kind: Option<DbKind>,
    report: &mut dyn FnMut(u64),
    cancel: &AtomicBool,
) -> Result<BackupIndex> {
    let kind = match kind {
        Some(k) => k,
        None => detect(input.fill_buf()?),
    };
    let mut b = Builder::new(kind);
    let mut sp = Splitter::new(kind, true);
    let mut line = Vec::new();
    let mut reported = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        line.clear();
        let n = input.read_until(b'\n', &mut line)?;
        let statements: Vec<RawStatement> = if n == 0 {
            sp.finish_raw().into_iter().collect()
        } else {
            sp.feed(&line)
        };
        for st in statements {
            if let Some((t, cols)) = b.statement(&st) {
                // The rows follow, up to `\.`.
                let data = sp.offset();
                let mut rows = 0;
                loop {
                    line.clear();
                    let n = input.read_until(b'\n', &mut line)?;
                    if n == 0 {
                        break;
                    }
                    sp.skip_line(n as u64);
                    if dump::trim_line(&line) == b"\\." {
                        break;
                    }
                    rows += 1;
                }
                b.add_rows(t, &st, Some(data), rows, cols);
            }
        }
        if n == 0 {
            break;
        }
        if sp.offset() - reported >= 4 << 20 {
            reported = sp.offset();
            report(reported);
        }
    }
    report(sp.offset());
    Ok(b.finish(sp.offset()))
}

/// One line of COPY text format as values (`\N` is NULL).
fn copy_values(line: &[u8]) -> Vec<Value> {
    let line = dump::trim_line(line);
    line.split(|c| *c == b'\t')
        .map(|f| {
            if f == b"\\N" {
                return Value::Null;
            }
            let mut out = Vec::with_capacity(f.len());
            let mut i = 0;
            while i < f.len() {
                if f[i] == b'\\' && i + 1 < f.len() {
                    i += 1;
                    match f[i] {
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'v' => out.push(11),
                        b'x' => {
                            let s = i + 1;
                            let mut e = s;
                            while e < f.len() && e < s + 2 && f[e].is_ascii_hexdigit() {
                                e += 1;
                            }
                            out.push(
                                u8::from_str_radix(
                                    std::str::from_utf8(&f[s..e]).unwrap_or("0"),
                                    16,
                                )
                                .unwrap_or(0),
                            );
                            i = e;
                            continue;
                        }
                        d @ b'0'..=b'7' => {
                            let mut n = u32::from(d - b'0');
                            let mut k = 0;
                            while k < 2 && i + 1 < f.len() && (b'0'..=b'7').contains(&f[i + 1]) {
                                i += 1;
                                k += 1;
                                n = n * 8 + u32::from(f[i] - b'0');
                            }
                            out.push(n as u8);
                        }
                        other => out.push(other),
                    }
                    i += 1;
                    continue;
                }
                out.push(f[i]);
                i += 1;
            }
            Value::from_bytes(&out)
        })
        .collect()
}

/// The names of `n` values: the chunk's or table's columns, numbered when they do not fit.
fn names_for(columns: &[String], n: usize) -> Vec<String> {
    if columns.len() == n {
        columns.to_vec()
    } else {
        (1..=n).map(|i| format!("column{i}")).collect()
    }
}

/// Every row of a table from position `from` (0-based, in file order), with its position;
/// `f` returns false to stop. Also hands each chunk's column names when one starts.
fn each_row(
    src: &mut dyn Source,
    ix: &BackupIndex,
    table: usize,
    from: u64,
    cancel: &AtomicBool,
    f: &mut RowFn,
) -> Result<()> {
    let t = ix
        .tables
        .get(table)
        .ok_or_else(|| Error::NotFound("table of the backup".into()))?;
    let mut pos = 0u64;
    for ch in &t.chunks {
        if pos + ch.rows <= from {
            pos += ch.rows;
            continue;
        }
        let cols = ch.columns.clone().unwrap_or_else(|| t.columns.clone());
        let mut input = src.open_at(ch.data.unwrap_or(ch.offset))?;
        let mut line = Vec::new();
        let mut left = ch.rows;
        if ch.data.is_some() {
            while left > 0 {
                if cancel.load(Ordering::Relaxed) {
                    return Err(cancelled());
                }
                line.clear();
                if input.read_until(b'\n', &mut line)? == 0 {
                    break;
                }
                left -= 1;
                if pos >= from {
                    let v = copy_values(&line);
                    let names = names_for(&cols, v.len());
                    if !f(pos, &names, v) {
                        return Ok(());
                    }
                }
                pos += 1;
            }
            continue;
        }
        let mut sp = Splitter::at(ix.kind, true, ch.offset, ch.line - 1);
        let mut stop = false;
        while left > 0 && !stop {
            if cancel.load(Ordering::Relaxed) {
                return Err(cancelled());
            }
            line.clear();
            let n = input.read_until(b'\n', &mut line)?;
            let statements: Vec<RawStatement> = if n == 0 {
                sp.finish_raw().into_iter().collect()
            } else {
                sp.feed(&line)
            };
            for st in statements {
                let mut c = Cur::new(&st.sql, ix.kind);
                if insert_head(&mut c).is_none() {
                    continue;
                }
                c.tuples(|v| {
                    if left == 0 {
                        return false;
                    }
                    left -= 1;
                    if pos >= from {
                        let names = names_for(&cols, v.len());
                        if !f(pos, &names, v) {
                            stop = true;
                            return false;
                        }
                    }
                    pos += 1;
                    true
                });
                if stop || left == 0 {
                    break;
                }
            }
            if n == 0 {
                break;
            }
        }
        if stop {
            return Ok(());
        }
    }
    Ok(())
}

fn matches(row: &[Value], filter: &str) -> bool {
    row.iter().any(|v| match v {
        Value::Text(s) => s.to_lowercase().contains(filter),
        Value::Bytes { hex } => hex.contains(filter),
        Value::Null => false,
    })
}

/// A page of a table's rows.
#[derive(Clone, Debug, Serialize)]
pub struct BackupPage {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    /// Positions of the rows in the table (file order).
    pub positions: Vec<u64>,
    /// Where the next page starts, if there are more rows.
    pub next: Option<u64>,
}

/// Up to `limit` rows from position `from` (with `filter`: only rows holding that text, any
/// case, and `next` is where the scan stopped).
pub fn rows(
    src: &mut dyn Source,
    ix: &BackupIndex,
    table: usize,
    from: u64,
    limit: usize,
    filter: Option<&str>,
    cancel: &AtomicBool,
) -> Result<BackupPage> {
    let filter = filter.map(str::to_lowercase).filter(|f| !f.is_empty());
    let mut page = BackupPage {
        columns: Vec::new(),
        rows: Vec::new(),
        positions: Vec::new(),
        next: None,
    };
    let total = ix.tables.get(table).map_or(0, |t| t.rows);
    let mut last = from;
    each_row(src, ix, table, from, cancel, &mut |pos, names, v| {
        last = pos + 1;
        if filter.as_deref().is_some_and(|f| !matches(&v, f)) {
            return true;
        }
        if page.rows.len() == limit {
            page.next = Some(pos);
            return false;
        }
        if page.columns.len() < names.len() {
            page.columns = names.to_vec();
        }
        page.rows.push(v);
        page.positions.push(pos);
        true
    })?;
    if page.next.is_none() && last < total && page.rows.len() == limit {
        page.next = Some(last);
    }
    if page.columns.is_empty() {
        page.columns = ix
            .tables
            .get(table)
            .map(|t| t.columns.clone())
            .unwrap_or_default();
    }
    Ok(page)
}

/// A table's CSV header: its columns, then any other a data statement of it names.
fn csv_header(t: &BackupTable) -> Vec<String> {
    let mut h = t.columns.clone();
    for ch in &t.chunks {
        for n in ch.columns.iter().flatten() {
            if !h.contains(n) {
                h.push(n.clone());
            }
        }
    }
    h
}

/// A row's values in `header`'s order: by name when the row's columns are named in the
/// header (a column the statement left out is NULL), else by position.
fn align(header: &[String], names: &[String], v: Vec<Value>) -> Vec<Value> {
    if names == header {
        return v;
    }
    if names.iter().all(|n| header.contains(n)) {
        return header
            .iter()
            .map(|h| {
                names
                    .iter()
                    .position(|n| n == h)
                    .map_or(Value::Null, |i| v[i].clone())
            })
            .collect();
    }
    let mut v = v;
    v.resize(header.len(), Value::Null);
    v
}

/// How a table is saved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Csv,
    Json,
    /// Structure and rows: the file's own statements (or INSERTs of the rows a filter kept).
    Sql,
}

fn csv_field(v: &Value) -> String {
    let s = match v {
        Value::Null => return String::new(),
        Value::Text(s) => s.clone(),
        Value::Bytes { hex } => format!("0x{hex}"),
    };
    if s.is_empty() || s.contains([',', '"', '\n', '\r']) || s.starts_with(' ') || s.ends_with(' ')
    {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s
    }
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_value(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Text(s) => json_string(s),
        Value::Bytes { hex } => json_string(&format!("0x{hex}")),
    }
}

fn quote(kind: DbKind, name: &str) -> String {
    match kind {
        DbKind::Mysql => format!("`{}`", name.replace('`', "``")),
        DbKind::Postgres => format!("\"{}\"", name.replace('"', "\"\"")),
    }
}

/// Write a statement read from the file back, with its delimiter.
fn put_statement(out: &mut dyn Write, s: &str) -> io::Result<()> {
    out.write_all(s.as_bytes())?;
    out.write_all(b";\n")
}

#[allow(clippy::too_many_arguments)]
/// Save a table (all of it, or the rows holding `filter`) as CSV, JSON or SQL. Returns the
/// rows written.
pub fn export(
    src: &mut dyn Source,
    ix: &BackupIndex,
    table: usize,
    format: Format,
    filter: Option<&str>,
    out: &mut dyn Write,
    report: &mut dyn FnMut(u64),
    cancel: &AtomicBool,
) -> Result<u64> {
    let t = ix
        .tables
        .get(table)
        .ok_or_else(|| Error::NotFound("table of the backup".into()))?
        .clone();
    let filter = filter.map(str::to_lowercase).filter(|f| !f.is_empty());
    let mut written = 0u64;
    let mut since = 0u32;
    let mut failed: Option<io::Error> = None;
    match format {
        Format::Csv | Format::Json => {
            let mut header: Option<Vec<String>> = None;
            // One header for every row: statements may name columns in another order.
            let fixed = csv_header(&t);
            if format == Format::Json {
                out.write_all(b"[")?;
            }
            each_row(src, ix, table, 0, cancel, &mut |_, names, v| {
                if filter.as_deref().is_some_and(|f| !matches(&v, f)) {
                    return true;
                }
                let line = match format {
                    Format::Csv => {
                        let mut s = String::new();
                        if header.is_none() {
                            let h = if fixed.is_empty() {
                                names.to_vec()
                            } else {
                                fixed.clone()
                            };
                            s.push_str(
                                &h.iter()
                                    .map(|n| csv_field(&Value::Text(n.clone())))
                                    .collect::<Vec<_>>()
                                    .join(","),
                            );
                            s.push_str("\r\n");
                            header = Some(h);
                        }
                        let row = align(header.as_deref().unwrap_or_default(), names, v);
                        s.push_str(&row.iter().map(csv_field).collect::<Vec<_>>().join(","));
                        s.push_str("\r\n");
                        s
                    }
                    _ => {
                        let fields: Vec<String> = names
                            .iter()
                            .zip(&v)
                            .map(|(n, v)| format!("{}: {}", json_string(n), json_value(v)))
                            .collect();
                        format!(
                            "{}\n  {{{}}}",
                            if written == 0 { "" } else { "," },
                            fields.join(", ")
                        )
                    }
                };
                if let Err(e) = out.write_all(line.as_bytes()) {
                    failed = Some(e);
                    return false;
                }
                written += 1;
                since += 1;
                if since == 10_000 {
                    since = 0;
                    report(written);
                }
                true
            })?;
            if let Some(e) = failed {
                return Err(e.into());
            }
            if format == Format::Json {
                out.write_all(if written == 0 { b"]\n" } else { b"\n]\n" })?;
            } else if header.is_none() {
                out.write_all(
                    t.columns
                        .iter()
                        .map(|n| csv_field(&Value::Text(n.clone())))
                        .collect::<Vec<_>>()
                        .join(",")
                        .as_bytes(),
                )?;
                out.write_all(b"\r\n")?;
            }
        }
        Format::Sql => {
            let name = match &t.schema {
                Some(s) if ix.kind == DbKind::Postgres => {
                    format!("{}.{}", quote(ix.kind, s), quote(ix.kind, &t.name))
                }
                _ => quote(ix.kind, &t.name),
            };
            writeln!(
                out,
                "-- Table {} from an SQL backup (Handrail)\n",
                t.name.replace('\n', " ")
            )?;
            for s in &ix.preamble {
                put_statement(out, s)?;
            }
            if let Some(d) = &t.drop {
                put_statement(out, d)?;
            }
            for s in &t.before {
                put_statement(out, s)?;
            }
            if let Some(c) = &t.create {
                put_statement(out, c)?;
            }
            if filter.is_none() {
                // The file's own statements: nothing is lost in translation.
                for ch in &t.chunks {
                    if cancel.load(Ordering::Relaxed) {
                        return Err(cancelled());
                    }
                    let mut input = src.open_at(ch.offset)?;
                    let mut sp = Splitter::at(ix.kind, true, ch.offset, ch.line - 1);
                    let mut left = ch.rows;
                    let mut line = Vec::new();
                    'chunk: loop {
                        line.clear();
                        let n = input.read_until(b'\n', &mut line)?;
                        let statements: Vec<RawStatement> = if n == 0 {
                            sp.finish_raw().into_iter().collect()
                        } else {
                            sp.feed(&line)
                        };
                        for st in statements {
                            out.write_all(&st.sql)?;
                            out.write_all(b";\n")?;
                            if ch.data.is_some() {
                                // COPY: its rows, then the end mark.
                                let mut rows = 0;
                                while rows < ch.rows {
                                    if cancel.load(Ordering::Relaxed) {
                                        return Err(cancelled());
                                    }
                                    line.clear();
                                    if input.read_until(b'\n', &mut line)? == 0 {
                                        break;
                                    }
                                    out.write_all(&line)?;
                                    if !line.ends_with(b"\n") {
                                        out.write_all(b"\n")?;
                                    }
                                    rows += 1;
                                }
                                out.write_all(b"\\.\n")?;
                                written += ch.rows;
                                break 'chunk;
                            }
                            let mut c = Cur::new(&st.sql, ix.kind);
                            if insert_head(&mut c).is_some() {
                                let n = c.tuples(|_| true).unwrap_or(0);
                                written += n;
                                left = left.saturating_sub(n);
                            }
                            if left == 0 {
                                break 'chunk;
                            }
                        }
                        if n == 0 {
                            break;
                        }
                    }
                    report(written);
                }
            } else {
                let mut ins: Option<(Vec<String>, dump::Inserts)> = None;
                each_row(src, ix, table, 0, cancel, &mut |_, names, v| {
                    if filter.as_deref().is_some_and(|f| !matches(&v, f)) {
                        return true;
                    }
                    if ins.as_ref().is_none_or(|(n, _)| n.as_slice() != names) {
                        if let Some((_, mut i)) = ins.take() {
                            if let Some(s) = i.finish() {
                                if let Err(e) = out.write_all(s.as_bytes()) {
                                    failed = Some(e);
                                    return false;
                                }
                            }
                        }
                        let cols: Vec<String> = names.iter().map(|n| quote(ix.kind, n)).collect();
                        ins = Some((names.to_vec(), dump::Inserts::new(&name, &cols, false)));
                    }
                    let lits: Vec<String> =
                        v.iter().map(|x| dump::literal(ix.kind, x, false)).collect();
                    if let Some((_, i)) = ins.as_mut() {
                        if let Some(s) = i.push(&lits) {
                            if let Err(e) = out.write_all(s.as_bytes()) {
                                failed = Some(e);
                                return false;
                            }
                        }
                    }
                    written += 1;
                    true
                })?;
                if let Some(e) = failed {
                    return Err(e.into());
                }
                if let Some((_, mut i)) = ins {
                    if let Some(s) = i.finish() {
                        out.write_all(s.as_bytes())?;
                    }
                }
            }
            for s in &t.after {
                put_statement(out, s)?;
            }
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(cancelled());
    }
    out.flush()?;
    report(written);
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Mem(Vec<u8>);
    impl Source for Mem {
        fn open_at(&mut self, offset: u64) -> io::Result<Box<dyn BufRead + '_>> {
            Ok(Box::new(&self.0[offset as usize..]))
        }
    }

    fn build(text: &[u8], kind: Option<DbKind>) -> (Mem, BackupIndex) {
        let stop = AtomicBool::new(false);
        let ix = index(&mut &text[..], kind, &mut |_| {}, &stop).unwrap();
        (Mem(text.to_vec()), ix)
    }

    fn texts(p: &BackupPage) -> Vec<Vec<String>> {
        p.rows
            .iter()
            .map(|r| {
                r.iter()
                    .map(|v| match v {
                        Value::Null => "NULL".into(),
                        Value::Text(s) => s.clone(),
                        Value::Bytes { hex } => format!("0x{hex}"),
                    })
                    .collect()
            })
            .collect()
    }

    const MYSQLDUMP: &[u8] = b"-- MySQL dump 10.13  Distrib 8.0.36\n\
/*!40101 SET NAMES utf8mb4 */;\n\
/*!40103 SET TIME_ZONE='+00:00' */;\n\
USE `shop`;\n\
DROP TABLE IF EXISTS `customers`;\n\
CREATE TABLE `customers` (\n  `id` int NOT NULL AUTO_INCREMENT,\n  `name` varchar(80) DEFAULT NULL,\n  `photo` blob,\n  PRIMARY KEY (`id`),\n  KEY `ix_name` (`name`)\n) ENGINE=InnoDB;\n\
LOCK TABLES `customers` WRITE;\n\
/*!40000 ALTER TABLE `customers` DISABLE KEYS */;\n\
INSERT INTO `customers` VALUES (1,'Ada \\'the\\' first','\xff\x00\\0'),(2,'semi;colon',NULL),(3,_binary 'x',0x4142);\n\
INSERT INTO `customers` VALUES (4,'line\\nbreak',X'00ff');\n\
/*!40000 ALTER TABLE `customers` ENABLE KEYS */;\n\
UNLOCK TABLES;\n\
/*!50001 DROP VIEW IF EXISTS `v_names`*/;\n\
/*!50001 CREATE ALGORITHM=UNDEFINED */\n/*!50013 DEFINER=`root`@`%` SQL SECURITY DEFINER */\n/*!50001 VIEW `v_names` AS select `customers`.`name` AS `name` from `customers` */;\n";

    #[test]
    fn indexes_and_reads_a_mysqldump() {
        assert_eq!(detect(MYSQLDUMP), DbKind::Mysql);
        let (mut src, ix) = build(MYSQLDUMP, None);
        let names: Vec<(&Option<String>, &str, bool, u64)> = ix
            .tables
            .iter()
            .map(|t| (&t.schema, t.name.as_str(), t.view, t.rows))
            .collect();
        assert_eq!(
            names,
            vec![
                (&Some("shop".to_string()), "customers", false, 4),
                (&Some("shop".to_string()), "v_names", true, 0)
            ]
        );
        let t = &ix.tables[0];
        assert_eq!(t.columns, vec!["id", "name", "photo"]);
        assert_eq!(t.chunks.len(), 1, "consecutive INSERTs share a chunk");
        assert!(
            t.drop.is_some()
                && t.create
                    .as_deref()
                    .unwrap()
                    .starts_with("CREATE TABLE `customers`")
        );
        assert_eq!(ix.preamble.len(), 2);
        let stop = AtomicBool::new(false);
        let p = rows(&mut src, &ix, 0, 0, 10, None, &stop).unwrap();
        assert_eq!(p.columns, vec!["id", "name", "photo"]);
        assert_eq!(
            texts(&p),
            vec![
                vec!["1", "Ada 'the' first", "0xff0000"],
                vec!["2", "semi;colon", "NULL"],
                vec!["3", "x", "AB"],
                vec!["4", "line\nbreak", "0x00ff"],
            ]
        );
        // Pages and filters.
        let p = rows(&mut src, &ix, 0, 1, 2, None, &stop).unwrap();
        assert_eq!(p.positions, vec![1, 2]);
        assert_eq!(p.next, Some(3));
        let p = rows(&mut src, &ix, 0, 0, 1, Some("BREAK"), &stop).unwrap();
        assert_eq!((p.positions.clone(), p.next), (vec![3], None));
        // A table saved as SQL: the file's statements; as CSV and JSON.
        let mut out = Vec::new();
        let n = export(
            &mut src,
            &ix,
            0,
            Format::Sql,
            None,
            &mut out,
            &mut |_| {},
            &stop,
        )
        .unwrap();
        assert_eq!(n, 4);
        let sql = String::from_utf8_lossy(&out);
        assert!(
            sql.contains("DROP TABLE IF EXISTS `customers`;\nCREATE TABLE `customers`"),
            "{sql}"
        );
        assert_eq!(
            sql.matches("INSERT INTO `customers` VALUES").count(),
            2,
            "{sql}"
        );
        assert!(sql.contains("/*!40101 SET NAMES utf8mb4 */;"));
        let mut out = Vec::new();
        export(
            &mut src,
            &ix,
            0,
            Format::Csv,
            Some("semi"),
            &mut out,
            &mut |_| {},
            &stop,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "id,name,photo\r\n2,semi;colon,\r\n"
        );
        let mut out = Vec::new();
        export(
            &mut src,
            &ix,
            0,
            Format::Json,
            Some("ada"),
            &mut out,
            &mut |_| {},
            &stop,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "[\n  {\"id\": \"1\", \"name\": \"Ada 'the' first\", \"photo\": \"0xff0000\"}\n]\n"
        );
        let mut out = Vec::new();
        export(
            &mut src,
            &ix,
            0,
            Format::Sql,
            Some("semi"),
            &mut out,
            &mut |_| {},
            &stop,
        )
        .unwrap();
        let sql = String::from_utf8_lossy(&out);
        assert!(sql.contains("INSERT INTO `customers` (`id`, `name`, `photo`) VALUES\n('2', 'semi;colon', NULL);"), "{sql}");
    }

    const PGDUMP: &[u8] = b"--\n-- PostgreSQL database dump\n--\n\\restrict k3y\n\
SET statement_timeout = 0;\nSET standard_conforming_strings = on;\n\
CREATE TABLE public.orders (\n    id integer NOT NULL,\n    note text,\n    \"Total\" numeric(10,2),\n    CONSTRAINT positive CHECK ((\"Total\" > 0))\n);\n\
ALTER TABLE public.orders OWNER TO app;\n\
CREATE SEQUENCE public.orders_id_seq\n    AS integer\n    START WITH 1;\n\
ALTER SEQUENCE public.orders_id_seq OWNED BY public.orders.id;\n\
ALTER TABLE ONLY public.orders ALTER COLUMN id SET DEFAULT nextval('public.orders_id_seq'::regclass);\n\
COPY public.orders (id, note, \"Total\") FROM stdin;\n\
1\tfirst\\ttab\t10.50\n\
2\t\\N\t3.00\n\
3\tsemi;colon\\nnewline\t7.25\n\
\\.\n\
INSERT INTO public.orders (id, note, \"Total\") VALUES (4, E'it''s\\x41', 1.5), (5, 'plain \\ backslash', NULL);\n\
SELECT pg_catalog.setval('public.orders_id_seq', 5, true);\n\
ALTER TABLE ONLY public.orders\n    ADD CONSTRAINT orders_pkey PRIMARY KEY (id);\n\
CREATE INDEX ix_note ON public.orders USING btree (note);\n\
\\unrestrict k3y\n";

    #[test]
    fn indexes_and_reads_a_pg_dump() {
        assert_eq!(detect(PGDUMP), DbKind::Postgres);
        let (mut src, ix) = build(PGDUMP, None);
        assert_eq!(ix.tables.len(), 1);
        let t = &ix.tables[0];
        assert_eq!(
            (t.schema.as_deref(), t.name.as_str(), t.rows),
            (Some("public"), "orders", 5)
        );
        assert_eq!(t.columns, vec!["id", "note", "Total"]);
        assert_eq!(t.chunks.len(), 2);
        let stop = AtomicBool::new(false);
        let p = rows(&mut src, &ix, 0, 0, 10, None, &stop).unwrap();
        assert_eq!(
            texts(&p),
            vec![
                vec!["1", "first\ttab", "10.50"],
                vec!["2", "NULL", "3.00"],
                vec!["3", "semi;colon\nnewline", "7.25"],
                vec!["4", "it'sA", "1.5"],
                vec!["5", "plain \\ backslash", "NULL"],
            ]
        );
        let p = rows(&mut src, &ix, 0, 2, 2, None, &stop).unwrap();
        assert_eq!((p.positions.clone(), p.next), (vec![2, 3], Some(4)));
        let mut out = Vec::new();
        let n = export(
            &mut src,
            &ix,
            0,
            Format::Sql,
            None,
            &mut out,
            &mut |_| {},
            &stop,
        )
        .unwrap();
        assert_eq!(n, 5);
        let sql = String::from_utf8_lossy(&out);
        let order = [
            "SET standard_conforming_strings = on;",
            "CREATE SEQUENCE public.orders_id_seq",
            "CREATE TABLE public.orders",
            "COPY public.orders (id, note, \"Total\") FROM stdin;\n1\tfirst\\ttab\t10.50\n2\t\\N\t3.00\n3\tsemi;colon\\nnewline\t7.25\n\\.\n",
            "INSERT INTO public.orders",
            "SET DEFAULT nextval('public.orders_id_seq'::regclass);",
            "ADD CONSTRAINT orders_pkey PRIMARY KEY (id);",
            "CREATE INDEX ix_note ON public.orders",
            "ALTER SEQUENCE public.orders_id_seq OWNED BY public.orders.id;",
            "setval('public.orders_id_seq', 5, true);",
        ];
        let mut at = 0;
        for o in order {
            let p = sql[at..]
                .find(o)
                .unwrap_or_else(|| panic!("{o} missing after {at}:\n{sql}"));
            at += p + o.len();
        }
        assert!(
            !sql.contains("OWNER TO") && !sql.contains("restrict"),
            "{sql}"
        );
        let mut out = Vec::new();
        export(
            &mut src,
            &ix,
            0,
            Format::Sql,
            Some("colon"),
            &mut out,
            &mut |_| {},
            &stop,
        )
        .unwrap();
        let sql = String::from_utf8_lossy(&out);
        assert!(sql.contains("INSERT INTO \"public\".\"orders\" (\"id\", \"note\", \"Total\") VALUES\n('3', 'semi;colon\nnewline', '7.25');"), "{sql}");
    }

    #[test]
    fn values_and_odd_statements() {
        let v = |s: &str, kind| {
            let mut c = Cur::new(s.as_bytes(), kind);
            let mut out = Vec::new();
            c.tuples(|r| {
                out.push(r);
                true
            });
            out
        };
        let m = v(
            "(-1.5e+3, 'a''b', NOW(), CONCAT('x', ','), TRUE, b'101', N'n')",
            DbKind::Mysql,
        );
        assert_eq!(
            m[0],
            vec![
                Value::Text("-1.5e+3".into()),
                Value::Text("a'b".into()),
                Value::Text("NOW()".into()),
                Value::Text("CONCAT('x', ',')".into()),
                Value::Text("TRUE".into()),
                Value::Text("b'101'".into()),
                Value::Text("n".into()),
            ]
        );
        let p = v("('\\x4142'::bytea, '2026-01-01'::timestamp with time zone, NULL::text, E'\\101\\u00e9')", DbKind::Postgres);
        assert_eq!(
            p[0],
            vec![
                Value::Text("AB".into()),
                Value::Text("2026-01-01".into()),
                Value::Null,
                Value::Text("A\u{e9}".into())
            ]
        );
        // Our own export: names qualified, rows on their own lines, a generated column left out.
        let ours = b"-- Handrail SQL dump\n-- Schema: s\nSET client_encoding = 'UTF8';\n\
CREATE TABLE \"s\".\"t\" (\n    \"id\" integer NOT NULL,\n    \"twice\" integer GENERATED ALWAYS AS ((id * 2)) STORED\n);\n\
INSERT INTO \"s\".\"t\" (\"id\") VALUES\n('1'),\n('2');\n";
        let (mut src, ix) = build(ours, None);
        assert_eq!(ix.kind, DbKind::Postgres);
        let stop = AtomicBool::new(false);
        let p = rows(&mut src, &ix, 0, 0, 10, None, &stop).unwrap();
        assert_eq!(p.columns, vec!["id"]);
        assert_eq!(texts(&p), vec![vec!["1"], vec!["2"]]);
        // Columns named in another order land under the right header; a column left out is
        // empty.
        let mixed = b"CREATE TABLE t (a int, b int, c int);\nINSERT INTO t (a, b) VALUES (1, 2);\nINSERT INTO t (b, a) VALUES (3, 4);\nINSERT INTO t VALUES (5, 6, 7);\n";
        let (mut src, ix) = build(mixed, Some(DbKind::Postgres));
        let stop = AtomicBool::new(false);
        let mut out = Vec::new();
        export(
            &mut src,
            &ix,
            0,
            Format::Csv,
            None,
            &mut out,
            &mut |_| {},
            &stop,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "a,b,c\r\n1,2,\r\n4,3,\r\n5,6,7\r\n"
        );
        // Same-named sequences in two schemas stay apart.
        let two = b"CREATE TABLE a.t (id int);\nCREATE TABLE b.t (id int);\n\
CREATE SEQUENCE a.t_id_seq;\nCREATE SEQUENCE b.t_id_seq;\n\
ALTER SEQUENCE a.t_id_seq OWNED BY a.t.id;\nALTER SEQUENCE b.t_id_seq OWNED BY b.t.id;\n\
SELECT pg_catalog.setval('a.t_id_seq', 7, true);\nSELECT pg_catalog.setval('b.t_id_seq', 9, true);\n";
        let (_, ix) = build(two, Some(DbKind::Postgres));
        let after = |i: usize| ix.tables[i].after.join(" ");
        assert!(
            after(0).contains("'a.t_id_seq', 7") && !after(0).contains("b.t_id_seq"),
            "{}",
            after(0)
        );
        assert!(
            after(1).contains("'b.t_id_seq', 9") && !after(1).contains("a.t_id_seq"),
            "{}",
            after(1)
        );
        assert_eq!(ix.tables[0].before, vec!["CREATE SEQUENCE a.t_id_seq"]);
        // Cancelled: an error, not a half index.
        let stop = AtomicBool::new(true);
        assert!(index(&mut &ours[..], None, &mut |_| {}, &stop).is_err());
    }
}
