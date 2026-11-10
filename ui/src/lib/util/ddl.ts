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
