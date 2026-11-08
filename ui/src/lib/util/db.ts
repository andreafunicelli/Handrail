import type { DbQueryOutput, DbResultSet, DbValue } from "../api/types";

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
