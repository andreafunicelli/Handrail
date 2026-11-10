/** SQL for changes to a database's structure, built here so the user sees it before it runs.
 *  Names are always quoted for the dialect; nothing the user types is pasted into the SQL
 *  unquoted except column types and default expressions (which are SQL by nature). */

import type { SqlDialect } from "./sqltext";

export function quoteIdent(d: SqlDialect, name: string): string {
  return d === "mysql" ? `\`${name.replace(/`/g, "``")}\`` : `"${name.replace(/"/g, '""')}"`;
}

export function qualified(d: SqlDialect, ns: string, table: string): string {
  return `${quoteIdent(d, ns)}.${quoteIdent(d, table)}`;
}

/** MySQL calls a namespace a database, PostgreSQL a schema. */
export const namespaceWord = (d: SqlDialect) => (d === "mysql" ? "DATABASE" : "SCHEMA");

export const createNamespaceSql = (d: SqlDialect, name: string) => `CREATE ${namespaceWord(d)} ${quoteIdent(d, name)};`;
export const dropNamespaceSql = (d: SqlDialect, name: string) => `DROP ${namespaceWord(d)} ${quoteIdent(d, name)};`;

/** What kind of relation a statement is about. */
export type RelKind = "table" | "view" | "materialized";
const relWord = (k: RelKind) => (k === "materialized" ? "MATERIALIZED VIEW" : k === "view" ? "VIEW" : "TABLE");

export const dropTableSql = (d: SqlDialect, ns: string, table: string, kind: RelKind) => `DROP ${relWord(kind)} ${qualified(d, ns, table)};`;
export const truncateTableSql = (d: SqlDialect, ns: string, table: string) => `TRUNCATE TABLE ${qualified(d, ns, table)};`;

export function renameTableSql(d: SqlDialect, ns: string, table: string, to: string, kind: RelKind = "table"): string {
  if (d === "mysql") return `RENAME TABLE ${qualified(d, ns, table)} TO ${qualified(d, ns, to)};`;
  return `ALTER ${relWord(kind)} ${qualified(d, ns, table)} RENAME TO ${quoteIdent(d, to)};`;
}

export const relKind = (t: { view: boolean; materialized?: boolean }): RelKind => (t.materialized ? "materialized" : t.view ? "view" : "table");

/** A name a user typed for a new table or namespace: trimmed, non-empty, one line. */
export function validName(name: string): boolean {
  const n = name.trim();
  return n.length > 0 && n.length <= 63 && !/[\r\n\0]/.test(n);
}

// ----- editing a table's structure: the edited copy is compared with what the server reported.

export interface ColDef {
  /** The column's name on the server; null for a new column. */
  orig: string | null;
  name: string;
  type: string;
  nullable: boolean;
  /** An SQL expression (a quoted literal, a number, CURRENT_TIMESTAMP, NULL...); null for none. */
  default: string | null;
  comment: string | null;
  /** MySQL collation, restated on CHANGE COLUMN so it is not reset. */
  collation?: string | null;
  /** What the server reported beyond type and default (auto_increment, identity, generated):
   *  kept as is. A generated column cannot be edited here. */
  extra: string | null;
}

export interface IdxDef {
  /** Existing indexes are kept or dropped, never changed in place. */
  orig: string | null;
  name: string;
  columns: string[];
  unique: boolean;
  primary: boolean;
}

export interface FkDef {
  orig: string | null;
  name: string;
  columns: string[];
  refNs: string;
  refTable: string;
  refColumns: string[];
  onUpdate: string;
  onDelete: string;
}

export interface TableDef {
  columns: ColDef[];
  indexes: IdxDef[];
  fks: FkDef[];
}

export const FK_RULES = ["NO ACTION", "RESTRICT", "CASCADE", "SET NULL", "SET DEFAULT"];

const lit = (d: SqlDialect, s: string) => `'${(d === "mysql" ? s.replace(/\\/g, "\\\\") : s).replace(/'/g, "''")}'`;

/** A generated column (its definition is an expression this editor does not change). */
export const isGenerated = (c: Pick<ColDef, "extra">) => !!c.extra && /GENERATED/i.test(c.extra) && !/IDENTITY/i.test(c.extra) && !/DEFAULT_GENERATED/i.test(c.extra);

/** MySQL's column definition: type, null, default, auto-increment, comment. */
function mysqlColumn(c: ColDef): string {
  let s = `${quoteIdent("mysql", c.name)} ${c.type.trim()}`;
  if (c.collation) s += ` COLLATE ${c.collation}`;
  s += c.nullable ? " NULL" : " NOT NULL";
  if (c.default !== null && c.default.trim() !== "") s += ` DEFAULT ${c.default.trim()}`;
  if (c.extra && /auto_increment/i.test(c.extra)) s += " AUTO_INCREMENT";
  if (c.extra && /on update current_timestamp/i.test(c.extra)) s += " ON UPDATE CURRENT_TIMESTAMP";
  if (c.comment) s += ` COMMENT ${lit("mysql", c.comment)}`;
  return s;
}

/** PostgreSQL's column definition for CREATE / ADD COLUMN (comments are separate statements). */
function pgColumn(c: ColDef): string {
  let s = `${quoteIdent("postgres", c.name)} ${c.type.trim()}`;
  if (c.extra && /IDENTITY/i.test(c.extra)) s += ` ${c.extra}`;
  else if (c.default !== null && c.default.trim() !== "") s += ` DEFAULT ${c.default.trim()}`;
  if (!c.nullable && !(c.extra && /IDENTITY/i.test(c.extra))) s += " NOT NULL";
  return s;
}

const cols = (d: SqlDialect, names: string[]) => names.map((n) => quoteIdent(d, n)).join(", ");

function fkClause(d: SqlDialect, f: FkDef): string {
  return `CONSTRAINT ${quoteIdent(d, f.name)} FOREIGN KEY (${cols(d, f.columns)}) REFERENCES ${qualified(d, f.refNs, f.refTable)} (${cols(d, f.refColumns)}) ON UPDATE ${f.onUpdate} ON DELETE ${f.onDelete}`;
}

/** Why a definition cannot be turned into SQL, or null when it can. */
export function defProblem(t: TableDef): string | null {
  const names = t.columns.map((c) => c.name.trim());
  if (!names.length) return "noColumns";
  if (names.some((n) => !validName(n)) || t.columns.some((c) => !c.type.trim())) return "columnIncomplete";
  if (new Set(names.map((n) => n.toLowerCase())).size !== names.length) return "columnTwice";
  const known = new Set(names);
  for (const i of t.indexes.filter((x) => x.orig === null)) {
    if ((!i.primary && !validName(i.name)) || !i.columns.length || i.columns.some((c) => !known.has(c))) return "indexIncomplete";
  }
  if (t.indexes.filter((i) => i.primary).length > 1) return "twoPrimary";
  for (const f of t.fks.filter((x) => x.orig === null)) {
    if (!validName(f.name) || !f.columns.length || f.columns.length !== f.refColumns.length || !f.refTable || f.columns.some((c) => !known.has(c))) return "fkIncomplete";
  }
  return null;
}

/** The CREATE TABLE statement (and, on PostgreSQL, the comments and indexes after it). */
export function createTableSql(d: SqlDialect, ns: string, table: string, t: TableDef): string {
  const target = qualified(d, ns, table);
  const lines = t.columns.map((c) => `  ${d === "mysql" ? mysqlColumn(c) : pgColumn(c)}`);
  const pk = t.indexes.find((i) => i.primary);
  if (pk) lines.push(`  PRIMARY KEY (${cols(d, pk.columns)})`);
  const after: string[] = [];
  for (const i of t.indexes.filter((x) => !x.primary)) {
    if (d === "mysql") lines.push(`  ${i.unique ? "UNIQUE " : ""}KEY ${quoteIdent(d, i.name)} (${cols(d, i.columns)})`);
    else after.push(`CREATE ${i.unique ? "UNIQUE " : ""}INDEX ${quoteIdent(d, i.name)} ON ${target} (${cols(d, i.columns)});`);
  }
  for (const f of t.fks) lines.push(`  ${fkClause(d, f)}`);
  if (d === "postgres") for (const c of t.columns) if (c.comment) after.push(`COMMENT ON COLUMN ${target}.${quoteIdent(d, c.name)} IS ${lit(d, c.comment)};`);
  return [`CREATE TABLE ${target} (\n${lines.join(",\n")}\n);`, ...after].join("\n");
}

/** The statements that turn `before` into `after`: foreign keys and indexes dropped first,
 *  then columns dropped, changed and added, then new indexes and foreign keys. Empty when
 *  nothing changed. */
export function alterTableSql(d: SqlDialect, ns: string, table: string, before: TableDef, after: TableDef): string[] {
  const target = qualified(d, ns, table);
  const out: string[] = [];
  const kept = <T extends { orig: string | null }>(list: T[]) => new Set(list.filter((x) => x.orig !== null).map((x) => x.orig));
  // Foreign keys and indexes that are gone.
  const fkKept = kept(after.fks);
  for (const f of before.fks) {
    if (!fkKept.has(f.name)) out.push(`ALTER TABLE ${target} DROP ${d === "mysql" ? "FOREIGN KEY" : "CONSTRAINT"} ${quoteIdent(d, f.name)};`);
  }
  const idxKept = kept(after.indexes);
  for (const i of before.indexes) {
    if (idxKept.has(i.name)) continue;
    if (i.primary) out.push(d === "mysql" ? `ALTER TABLE ${target} DROP PRIMARY KEY;` : `ALTER TABLE ${target} DROP CONSTRAINT ${quoteIdent(d, i.name)};`);
    else if (d === "mysql") out.push(`DROP INDEX ${quoteIdent(d, i.name)} ON ${target};`);
    // A PostgreSQL index that backs a UNIQUE constraint is dropped with the constraint.
    else if (i.unique) out.push(`ALTER TABLE ${target} DROP CONSTRAINT IF EXISTS ${quoteIdent(d, i.name)};\nDROP INDEX IF EXISTS ${qualified(d, ns, i.name)};`);
    else out.push(`DROP INDEX ${qualified(d, ns, i.name)};`);
  }
  // Columns.
  const colKept = kept(after.columns);
  for (const c of before.columns) if (!colKept.has(c.name)) out.push(`ALTER TABLE ${target} DROP COLUMN ${quoteIdent(d, c.name)};`);
  for (const c of after.columns) {
    const was = c.orig === null ? undefined : before.columns.find((b) => b.name === c.orig);
    if (!was) {
      out.push(`ALTER TABLE ${target} ADD COLUMN ${d === "mysql" ? mysqlColumn(c) : pgColumn(c)};`);
      if (d === "postgres" && c.comment) out.push(`COMMENT ON COLUMN ${target}.${quoteIdent(d, c.name)} IS ${lit(d, c.comment)};`);
      continue;
    }
    const same = (a: string | null, b: string | null) => (a ?? "").trim() === (b ?? "").trim();
    const changed = c.name !== was.name || !same(c.type, was.type) || c.nullable !== was.nullable || !same(c.default, was.default) || !same(c.comment, was.comment);
    if (!changed) continue;
    if (d === "mysql") {
      out.push(`ALTER TABLE ${target} CHANGE COLUMN ${quoteIdent(d, was.name)} ${mysqlColumn(c)};`);
      continue;
    }
    const col = quoteIdent(d, c.name);
    if (c.name !== was.name) out.push(`ALTER TABLE ${target} RENAME COLUMN ${quoteIdent(d, was.name)} TO ${col};`);
    if (!same(c.type, was.type)) out.push(`ALTER TABLE ${target} ALTER COLUMN ${col} TYPE ${c.type.trim()} USING ${col}::${c.type.trim()};`);
    if (c.nullable !== was.nullable) out.push(`ALTER TABLE ${target} ALTER COLUMN ${col} ${c.nullable ? "DROP" : "SET"} NOT NULL;`);
    if (!same(c.default, was.default)) out.push(c.default?.trim() ? `ALTER TABLE ${target} ALTER COLUMN ${col} SET DEFAULT ${c.default.trim()};` : `ALTER TABLE ${target} ALTER COLUMN ${col} DROP DEFAULT;`);
    if (!same(c.comment, was.comment)) out.push(`COMMENT ON COLUMN ${target}.${col} IS ${c.comment?.trim() ? lit(d, c.comment.trim()) : "NULL"};`);
  }
  // New indexes and foreign keys.
  for (const i of after.indexes.filter((x) => x.orig === null)) {
    if (i.primary) out.push(`ALTER TABLE ${target} ADD PRIMARY KEY (${cols(d, i.columns)});`);
    else out.push(`CREATE ${i.unique ? "UNIQUE " : ""}INDEX ${quoteIdent(d, i.name)} ON ${target} (${cols(d, i.columns)});`);
  }
  for (const f of after.fks.filter((x) => x.orig === null)) out.push(`ALTER TABLE ${target} ADD ${fkClause(d, f)};`);
  return out;
}

/** What the server reported as a column default, as an SQL expression the editor can show and
 *  send back. MySQL 8 reports a literal default bare (`abc`) and marks expressions
 *  DEFAULT_GENERATED; MariaDB reports SQL already (`'abc'`, `NULL`); PostgreSQL too. */
export function defaultAsSql(d: SqlDialect, mariadb: boolean, value: string | null, extra: string | null, type: string): string | null {
  if (value === null) return null;
  if (d === "postgres" || mariadb) return value === "NULL" ? null : value;
  // MySQL 5.7 does not mark these as expressions.
  if (/^(CURRENT_TIMESTAMP|CURRENT_DATE|CURRENT_TIME|LOCALTIME|LOCALTIMESTAMP|NOW)(\(\d*\))?$/i.test(value)) return value;
  if (extra && /DEFAULT_GENERATED/i.test(extra)) return /^\(.*\)$/s.test(value) || /^[A-Z_]+(\(\d*\))?$/i.test(value) ? value : `(${value})`;
  if (/^(tiny|small|medium|big)?int|decimal|numeric|float|double|bit|year/i.test(type) && /^-?\d+(\.\d+)?$/.test(value)) return value;
  return lit("mysql", value);
}
