import { describe, expect, it } from "vitest";
import { addHistory, clearHistory, HISTORY_MAX, loadHistory, loadTabs, nextTabName, saveTabs } from "./dbtabs";

class Mem implements Storage {
  m = new Map<string, string>();
  get length() {
    return this.m.size;
  }
  clear() {
    this.m.clear();
  }
  getItem(k: string) {
    return this.m.get(k) ?? null;
  }
  key(i: number) {
    return [...this.m.keys()][i] ?? null;
  }
  removeItem(k: string) {
    this.m.delete(k);
  }
  setItem(k: string, v: string) {
    this.m.set(k, v);
  }
}

describe("query tabs and history", () => {
  it("starts from the old single editor text, then keeps the saved tabs", () => {
    const s = new Mem();
    s.setItem("handrail.db.sql.c1", "SELECT 1");
    let n = 0;
    const key = () => `k${++n}`;
    expect(loadTabs(s, "c1", key)).toEqual({ tabs: [{ key: "k1", name: "Query 1", sql: "SELECT 1" }], current: 0 });
    const tabs = [
      { key: "a", name: "Query 1", sql: "x" },
      { key: "b", name: "Query 2", sql: "y" },
    ];
    saveTabs(s, "c1", tabs, 7);
    expect(loadTabs(s, "c1", key)).toEqual({ tabs, current: 1 });
    expect(nextTabName(tabs)).toBe("Query 3");
    expect(nextTabName([{ key: "c", name: "Query 2", sql: "" }])).toBe("Query 3");
    s.setItem("handrail.db.tabs.c2", "not json");
    expect(loadTabs(s, "c2", key).tabs[0].sql).toBe("");
  });

  it("keeps the newest history first, once per SQL, up to a limit", () => {
    const s = new Mem();
    addHistory(s, "c", { sql: "a", at: 1, ms: 1, ok: true });
    addHistory(s, "c", { sql: "b", at: 2, ms: 1, ok: false });
    const list = addHistory(s, "c", { sql: "a", at: 3, ms: 1, ok: true });
    expect(list.map((e) => e.sql)).toEqual(["a", "b"]);
    for (let i = 0; i < HISTORY_MAX + 5; i++) addHistory(s, "c", { sql: `q${i}`, at: i, ms: 0, ok: true });
    expect(loadHistory(s, "c")).toHaveLength(HISTORY_MAX);
    clearHistory(s, "c");
    expect(loadHistory(s, "c")).toEqual([]);
  });
});
