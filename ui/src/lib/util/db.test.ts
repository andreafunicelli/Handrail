import { describe, expect, it } from "vitest";
import type { DbResultSet } from "../api/types";
import { cellText, columnWidths, primaryResult, toCsv } from "./db";

const set = (columns: string[], rows: DbResultSet["rows"], extra: Partial<DbResultSet> = {}): DbResultSet => ({
  columns: columns.map((name) => ({ name, type_name: "TEXT" })),
  rows,
  rows_affected: 0,
  truncated: false,
  ...extra,
});

describe("db helpers", () => {
  it("shows cells as text", () => {
    expect(cellText(null)).toBe("");
    expect(cellText("x")).toBe("x");
    expect(cellText({ hex: "ff00" })).toBe("0xff00");
  });

  it("writes CSV that spreadsheets read back", () => {
    const csv = toCsv(set(["id", "note"], [["1", 'say "hi", then\nleave'], ["2", null]]));
    expect(csv).toBe('id,note\r\n1,"say ""hi"", then\nleave"\r\n2,');
  });

  it("sizes columns within bounds", () => {
    const w = columnWidths(set(["a", "b"], [["1", "x".repeat(500)]]));
    expect(w[0]).toBeGreaterThanOrEqual(60);
    expect(w[1]).toBeLessThanOrEqual(360);
  });

  it("shows the last result with columns first", () => {
    const out = { results: [set(["a"], [["1"]]), set([], [], { rows_affected: 3 })], error: null, elapsed_ms: 1 };
    expect(primaryResult(out)).toBe(0);
    expect(primaryResult({ ...out, results: [set([], [])] })).toBe(0);
  });
});
