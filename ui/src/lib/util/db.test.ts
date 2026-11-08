import { describe, expect, it } from "vitest";
import type { DbResultSet } from "../api/types";
import { buildEdits, cellText, columnWidths, editCount, editedValue, noEdits, primaryResult, toCsv } from "./db";

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

  it("turns grid edits into row edits by primary key", () => {
    const r = set(["id", "name", "note"], [["1", "a", null], ["2", "b", "x"], ["3", "c", "y"]]);
    const e = noEdits();
    e.cells[0] = { 1: "A", 2: "note" };
    e.cells[2] = { 1: "gone anyway" };
    e.deleted[2] = true;
    e.added.push({ 1: "new" });
    expect(editCount(e)).toBe(3);
    expect(editedValue(r, e, 0, 1)).toBe("A");
    expect(editedValue(r, e, 1, 1)).toBe("b");
    expect(editedValue(r, e, 3, 1)).toBe("new");
    expect(editedValue(r, e, 3, 0)).toBeUndefined();
    expect(buildEdits(r, ["id"], e)).toEqual([
      { op: "delete", key: [["id", "3"]] },
      { op: "update", key: [["id", "1"]], set: [["name", "A"], ["note", "note"]] },
      { op: "insert", values: [["name", "new"]] },
    ]);
    const bin = set(["id"], [[{ hex: "00" }]]);
    const d = noEdits();
    d.deleted[0] = true;
    expect(() => buildEdits(bin, ["id"], d)).toThrow();
  });
});
