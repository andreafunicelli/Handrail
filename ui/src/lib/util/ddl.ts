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

export const dropTableSql = (d: SqlDialect, ns: string, table: string, view: boolean) => `DROP ${view ? "VIEW" : "TABLE"} ${qualified(d, ns, table)};`;
export const truncateTableSql = (d: SqlDialect, ns: string, table: string) => `TRUNCATE TABLE ${qualified(d, ns, table)};`;

export function renameTableSql(d: SqlDialect, ns: string, table: string, to: string, view = false): string {
  if (d === "mysql") return `RENAME TABLE ${qualified(d, ns, table)} TO ${qualified(d, ns, to)};`;
  return `ALTER ${view ? "VIEW" : "TABLE"} ${qualified(d, ns, table)} RENAME TO ${quoteIdent(d, to)};`;
}

/** A name a user typed for a new table or namespace: trimmed, non-empty, one line. */
export function validName(name: string): boolean {
  const n = name.trim();
  return n.length > 0 && n.length <= 63 && !/[\r\n\0]/.test(n);
}
