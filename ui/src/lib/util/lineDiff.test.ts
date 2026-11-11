import { describe, expect, it } from "vitest";
import { compareTexts, diffLines, splitLines } from "./lineDiff";

const ops = (a: string[], b: string[]) => diffLines(a, b)!.map((l) => `${l.op === "same" ? " " : l.op === "add" ? "+" : "-"}${l.text}`);

describe("splitLines", () => {
  it("drops the final newline, keeps empty lines, normalizes CRLF", () => {
    expect(splitLines("")).toEqual([]);
    expect(splitLines("a\nb\n")).toEqual(["a", "b"]);
    expect(splitLines("a\r\n\r\nb")).toEqual(["a", "", "b"]);
  });
});

describe("diffLines", () => {
  it("identical texts are all kept", () => {
    expect(ops(["a", "b"], ["a", "b"])).toEqual([" a", " b"]);
  });

  it("finds an insertion, a deletion and a change", () => {
    expect(ops(["a", "b", "c"], ["a", "x", "b", "c"])).toEqual([" a", "+x", " b", " c"]);
    expect(ops(["a", "b", "c"], ["a", "c"])).toEqual([" a", "-b", " c"]);
    expect(ops(["a", "b", "c"], ["a", "B", "c"])).toEqual([" a", "-b", "+B", " c"]);
  });

  it("keeps the longest common subsequence", () => {
    const out = diffLines(["a", "b", "c", "d", "e"], ["b", "c", "x", "e"])!;
    expect(out.filter((l) => l.op === "same").map((l) => l.text)).toEqual(["b", "c", "e"]);
  });

  it("numbers lines on both sides", () => {
    const out = diffLines(["a", "b"], ["z", "a", "b"])!;
    expect(out[0]).toEqual({ op: "add", text: "z", b: 1 });
    expect(out[1]).toEqual({ op: "same", text: "a", a: 1, b: 2 });
    expect(out[2]).toEqual({ op: "same", text: "b", a: 2, b: 3 });
  });

  it("from or to nothing", () => {
    expect(ops([], ["a"])).toEqual(["+a"]);
    expect(ops(["a"], [])).toEqual(["-a"]);
  });

  it("gives up past the work limit", () => {
    const a = Array.from({ length: 300 }, (_, i) => `a${i}`);
    const b = Array.from({ length: 300 }, (_, i) => `b${i}`);
    expect(diffLines(a, b, 10_000)).toBeNull();
    // The common start and end do not count.
    const same = Array.from({ length: 1000 }, (_, i) => `s${i}`);
    expect(diffLines([...same, "x", ...same], [...same, "y", ...same], 10)).not.toBeNull();
  });
});

describe("compareTexts", () => {
  it("groups changes into hunks with context and counts what is skipped", () => {
    const old = Array.from({ length: 20 }, (_, i) => `line ${i + 1}`).join("\n");
    const neu = old.replace("line 10", "line ten");
    const r = compareTexts(old, neu, { context: 2 });
    if (!r.ok) throw new Error("expected a diff");
    expect(r.added).toBe(1);
    expect(r.removed).toBe(1);
    expect(r.hunks[0].skipped).toBe(7);
    expect(r.hunks[0].lines.map((l) => l.text)).toEqual(["line 8", "line 9", "line 10", "line ten", "line 11", "line 12"]);
    expect(r.hunks[1]).toEqual({ skipped: 8, lines: [] });
  });

  it("identical and line-ending-only differences", () => {
    const same = compareTexts("a\nb\n", "a\nb\n");
    expect(same.ok && same.identical).toBe(true);
    const crlf = compareTexts("a\nb\n", "a\r\nb");
    expect(crlf.ok && !crlf.identical && crlf.added === 0 && crlf.removed === 0).toBe(true);
  });

  it("refuses texts with too many lines", () => {
    const big = "x\n".repeat(50);
    expect(compareTexts(big, "y", { maxLines: 10 })).toEqual({ ok: false, reason: "lines" });
  });
});
