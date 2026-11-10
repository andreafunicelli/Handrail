import type { DbCellValue, DbQueryOutput, DbResultSet, DbRowEdit, DbTableInfo, DbValue } from "../api/types";
import { formatBytes } from "./time";

/** A cell as text: NULL as an empty string, bytes as `0x…`. */
export function cellText(v: DbValue): string {
  if (v === null) return "";
  if (typeof v === "string") return v;
  return `0x${v.hex}`;
}

const CHAR = 7.4;

/** Column widths in pixels from the header and the first rows (60–360 px). */
export function columnWidths(result: DbResultSet, sample = 200): number[] {
  return result.columns.map((c, i) => {
    let longest = Math.max(c.name.length + c.type_name.length + 2, 4);
    for (const row of result.rows.slice(0, sample)) {
      const v = row[i];
      longest = Math.max(longest, v === null ? 4 : Math.min(cellText(v).length, 60));
    }
    return Math.round(Math.min(360, Math.max(60, longest * CHAR + 18)));
  });
}

/** RFC 4180 CSV of a result: fields with commas, quotes or line breaks are quoted. */
export function toCsv(result: DbResultSet): string {
  const field = (s: string) => (/[",\r\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s);
  const lines = [result.columns.map((c) => field(c.name)).join(",")];
  for (const row of result.rows) lines.push(row.map((v) => field(cellText(v))).join(","));
  return lines.join("\r\n");
}

/** The result to show first: the last one with rows or columns, else the last. */
export function primaryResult(out: DbQueryOutput): number {
  for (let i = out.results.length - 1; i >= 0; i--) if (out.results[i].columns.length) return i;
  return out.results.length - 1;
}

/** A one-line summary of a result: rows (and whether they were cut), or rows affected. */
export function resultSummary(r: DbResultSet): { rows: number; affected: number; truncated: boolean; isQuery: boolean } {
  return { rows: r.rows.length, affected: r.rows_affected, truncated: r.truncated, isQuery: r.columns.length > 0 };
}

/**
 * Edits made in the grid of one table's rows, not saved yet: changed cells of existing rows,
 * rows marked for deletion, and new rows (a missing cell takes the column's default).
 */
export interface GridEdits {
  cells: Record<number, Record<number, DbCellValue>>;
  deleted: Record<number, true>;
  added: Record<number, DbCellValue>[];
}

export const noEdits = (): GridEdits => ({ cells: {}, deleted: {}, added: [] });

/** How many rows the edits touch. */
export function editCount(e: GridEdits): number {
  const changed = Object.keys(e.cells).filter((r) => !e.deleted[Number(r)] && Object.keys(e.cells[Number(r)]).length).length;
  return changed + Object.keys(e.deleted).length + e.added.length;
}

/** A cell's value with the edits applied (`undefined`: a new row's cell left to its default). */
export function editedValue(result: DbResultSet, e: GridEdits, r: number, c: number): DbValue | undefined {
  if (r >= result.rows.length) return e.added[r - result.rows.length]?.[c];
  const row = e.cells[r];
  return row && c in row ? row[c] : result.rows[r][c];
}

/**
 * The row edits to send, identifying rows by `primaryKey` (column names). Fails when a key cell
 * is binary (it cannot be sent back as text).
 */
export function buildEdits(result: DbResultSet, primaryKey: string[], e: GridEdits): DbRowEdit[] {
  const names = result.columns.map((c) => c.name);
  const keyOf = (r: number): [string, DbCellValue][] =>
    primaryKey.map((k) => {
      const v = result.rows[r][names.indexOf(k)];
      if (v !== null && typeof v !== "string") throw new Error(`the key column ${k} holds binary data`);
      return [k, v];
    });
  const out: DbRowEdit[] = [];
  for (const r of Object.keys(e.deleted).map(Number)) out.push({ op: "delete", key: keyOf(r) });
  for (const [r, cells] of Object.entries(e.cells)) {
    const row = Number(r);
    if (e.deleted[row]) continue;
    const set = Object.entries(cells).map(([c, v]) => [names[Number(c)], v] as [string, DbCellValue]);
    if (set.length) out.push({ op: "update", key: keyOf(row), set });
  }
  for (const row of e.added) {
    out.push({ op: "insert", values: Object.entries(row).map(([c, v]) => [names[Number(c)], v] as [string, DbCellValue]) });
  }
  return out;
}

/** "~1,234 rows · 4.2 MB · InnoDB" for a table, from the server's statistics. */
export function tableStats(t: DbTableInfo): string {
  const parts: string[] = [];
  if (t.rows !== null) parts.push(`~${t.rows.toLocaleString()} rows`);
  if (t.size !== null) parts.push(formatBytes(t.size));
  if (t.engine) parts.push(t.engine);
  return parts.join(" · ");
}

/** A namespace's tables as a result set (the database overview). */
export function tableOverview(list: DbTableInfo[]): DbResultSet {
  const text = (name: string) => ({ name, type_name: "TEXT" });
  return {
    columns: [text("name"), text("type"), { name: "rows (estimate)", type_name: "BIGINT" }, text("size"), text("engine"), text("comment")],
    rows: list.map((t) => [
      t.name,
      t.view ? "view" : "table",
      t.rows === null ? null : String(t.rows),
      t.size === null ? null : formatBytes(t.size),
      t.engine,
      t.comment,
    ]),
    rows_affected: 0,
    truncated: false,
  };
}

/** Rows as a JSON array of objects (binary values as `0x…` hex text). */
export function toJson(result: DbResultSet): string {
  const names = result.columns.map((c) => c.name);
  const rows = result.rows.map((row) => Object.fromEntries(names.map((n, i) => [n, row[i] === null ? null : cellText(row[i])])));
  return JSON.stringify(rows, null, 2);
}

/** Rows as INSERT statements into the table named by `path` (e.g. `["shop", "orders"]`). */
export function toInserts(result: DbResultSet, path: string[], dialect: "mysql" | "postgres"): string {
  const q = (n: string) => (dialect === "mysql" ? `\`${n.replace(/`/g, "``")}\`` : `"${n.replace(/"/g, '""')}"`);
  const target = path.map(q).join(".");
  const cols = result.columns.map((c) => q(c.name)).join(", ");
  const literal = (v: DbValue): string => {
    if (v === null) return "NULL";
    if (typeof v !== "string") return dialect === "mysql" ? `X'${v.hex}'` : `'\\x${v.hex}'::bytea`;
    let s = v.replace(/'/g, "''");
    // MySQL reads backslashes in strings as escapes (unless NO_BACKSLASH_ESCAPES).
    if (dialect === "mysql") s = s.replace(/\\/g, "\\\\");
    return `'${s}'`;
  };
  return result.rows.map((row) => `INSERT INTO ${target} (${cols}) VALUES (${row.map(literal).join(", ")});`).join("\n") + (result.rows.length ? "\n" : "");
}
