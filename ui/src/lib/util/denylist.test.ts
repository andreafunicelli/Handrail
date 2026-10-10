import { describe, expect, it } from "vitest";
import { escapeRegex, literalText, normalizeDenyList, parseDenyInput } from "./denylist";

describe("deny-list input", () => {
  it("stores plain text as an escaped literal", () => {
    expect(parseDenyInput("  rm -rf  ")).toEqual({ ok: true, pattern: "rm \\-rf" });
    expect(parseDenyInput("kill -9 $(pidof x)")).toEqual({ ok: true, pattern: "kill \\-9 \\$\\(pidof x\\)" });
    // A path with one leading slash is text, not /regex/.
    expect(parseDenyInput("/etc/shadow")).toEqual({ ok: true, pattern: "/etc/shadow" });
    expect(parseDenyInput("   ")).toEqual({ ok: false, error: "empty" });
  });

  it("turns /regex/flags into a Rust regex with inline flags", () => {
    expect(parseDenyInput("/drop\\s+table/i")).toEqual({ ok: true, pattern: "(?i)drop\\s+table" });
    expect(parseDenyInput("/^reboot$/gm")).toEqual({ ok: true, pattern: "(?m)^reboot$" });
    expect(parseDenyInput("/rm\\s+\\/var/")).toEqual({ ok: true, pattern: "rm\\s+/var" });
    expect(parseDenyInput("/a/si")).toEqual({ ok: true, pattern: "(?is)a" });
    expect(parseDenyInput("/(?i)drop/")).toEqual({ ok: true, pattern: "(?i)drop" });
  });

  it("rejects what the Rust engine would silently drop", () => {
    expect(parseDenyInput("/foo(/")).toEqual({ ok: false, error: "invalid" });
    expect(parseDenyInput("/rm(?!-i)/")).toEqual({ ok: false, error: "unsupported" });
    expect(parseDenyInput("/(a)\\1/")).toEqual({ ok: false, error: "unsupported" });
    // Unknown "flags" mean it was a path after all.
    expect(parseDenyInput("/usr/bin")).toEqual({ ok: true, pattern: "/usr/bin" });
  });

  it("round-trips literals for display", () => {
    for (const s of ["rm -rf", "a.b*c", "C:\\Windows", "x [y] {z} ^$ | ? + #&~"]) {
      expect(literalText(escapeRegex(s))).toBe(s);
      expect(new RegExp(escapeRegex(s).replace(/\\([#&~-])/g, "$1")).test(`pre ${s} post`)).toBe(true);
    }
    expect(literalText("rm -rf")).toBe("rm -rf");
    expect(literalText("(?i)drop\\s+table")).toBeNull();
    expect(literalText("\\bmkfs\\b")).toBeNull();
  });

  it("converts legacy /regex/ entries and keeps the array when nothing changes", () => {
    const clean = ["rm \\-rf", "(?i)drop"];
    expect(normalizeDenyList(clean)).toBe(clean);
    expect(normalizeDenyList(["/drop\\s+table/i", "(?i)drop\\s+table", "/bad(/"])).toEqual(["(?i)drop\\s+table", "/bad(/"]);
  });
});
