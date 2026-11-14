import { describe, expect, it } from "vitest";
import { ARM_MS, armed, effectiveMode, lineBreaks, revealHidden, takesText } from "./approval";

const el = (tagName: string, extra: Record<string, unknown> = {}) => ({ tagName, ...extra }) as unknown as Element;

describe("approval guards", () => {
  it("ignores accepting keys right after the prompt appears", () => {
    expect(armed(1000, 1000 + ARM_MS - 1)).toBe(false);
    expect(armed(1000, 1000 + ARM_MS)).toBe(true);
  });

  it("knows where the user is typing", () => {
    expect(takesText(el("TEXTAREA"))).toBe(true);
    expect(takesText(el("INPUT", { type: "text" }))).toBe(true);
    expect(takesText(el("INPUT", { type: "search" }))).toBe(true);
    expect(takesText(el("INPUT", { type: "checkbox" }))).toBe(false);
    expect(takesText(el("DIV", { isContentEditable: true }))).toBe(true);
    expect(takesText(el("BUTTON", { isContentEditable: false }))).toBe(false);
    expect(takesText(null)).toBe(false);
  });

  it("shows hidden and direction-changing characters", () => {
    expect(revealHidden("ls -la")).toEqual({ text: "ls -la", found: false });
    expect(revealHidden("echo ‮exe.sh")).toEqual({ text: "echo ⟨U+202E⟩exe.sh", found: true });
    expect(revealHidden("rm​ -rf⁦x⁩").text).toBe("rm⟨U+200B⟩ -rf⟨U+2066⟩x⟨U+2069⟩");
    expect(revealHidden("a\u{E0041}b").text).toBe("a⟨U+E0041⟩b");
    expect(revealHidden("DROP﻿ TABLE").found).toBe(true);
  });

  it("counts line breaks of pasted text", () => {
    expect(lineBreaks("ls")).toBe(0);
    expect(lineBreaks("ls\n")).toBe(1);
    expect(lineBreaks("a\r\nb\rc\n")).toBe(3);
  });

  it("knows Auto runs as Ask writes on production", () => {
    expect(effectiveMode("prod", "auto")).toBe("confirm_writes");
    expect(effectiveMode("staging", "auto")).toBe("auto");
    expect(effectiveMode("prod", "confirm_all")).toBe("confirm_all");
  });
});
