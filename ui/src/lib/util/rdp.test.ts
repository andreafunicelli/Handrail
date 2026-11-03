import { describe, expect, it, vi } from "vitest";
import type { RdpInput } from "../api/types";
import { MAX_PASTE_BYTES, MSG_ERROR, MSG_POINTER, MSG_TERMINATED, PASTE_WAIT_MS, RdpKeyboard, encodeTiles, fitPaste, keyboardPlatform, parseMessage, scancode, WheelAccumulator } from "./rdp";

const bytesOf = (kind: number, payload: number[] | string) => {
  const body = typeof payload === "string" ? [...new TextEncoder().encode(payload)] : payload;
  return new Uint8Array([kind, ...body]).buffer;
};

describe("rdp messages", () => {
  it("round-trips tiles", () => {
    const buf = encodeTiles(4, 3, [{ x: 1, y: 1, w: 2, h: 2 }], (x, y) => [x, y, 7]);
    const m = parseMessage(buf);
    if (m.kind !== "tiles") throw new Error(m.kind);
    expect([m.tiles.width, m.tiles.height]).toEqual([4, 3]);
    expect(m.tiles.rects.map(({ x, y, w, h }) => [x, y, w, h])).toEqual([[1, 1, 2, 2]]);
    expect([...m.tiles.rects[0].data]).toEqual([1, 1, 7, 255, 2, 1, 7, 255, 1, 2, 7, 255, 2, 2, 7, 255]);
  });

  it("refuses truncated tiles", () => {
    const buf = encodeTiles(4, 3, [{ x: 0, y: 0, w: 4, h: 3 }], () => [0, 0, 0]);
    expect(() => parseMessage(buf.slice(0, buf.byteLength - 1))).toThrow();
  });

  it("reads pointers, errors and endings", () => {
    expect(parseMessage(bytesOf(MSG_POINTER, [0]))).toEqual({ kind: "pointer", pointer: { kind: "default" } });
    expect(parseMessage(bytesOf(MSG_POINTER, [1]))).toEqual({ kind: "pointer", pointer: { kind: "hidden" } });
    const bmp = parseMessage(bytesOf(MSG_POINTER, [3, 1, 0, 1, 0, 0, 0, 0, 0, 9, 8, 7, 6]));
    expect(bmp.kind === "pointer" && bmp.pointer.kind === "bitmap" && [...bmp.pointer.rgba]).toEqual([9, 8, 7, 6]);
    expect(parseMessage(bytesOf(MSG_ERROR, '{"message":"refused"}'))).toEqual({ kind: "error", message: "refused" });
    expect(parseMessage(bytesOf(MSG_TERMINATED, '{"reason":"server initiated disconnect","error":false}'))).toEqual({
      kind: "terminated",
      reason: "server initiated disconnect",
      error: false,
    });
    expect(parseMessage(bytesOf(0x7f, [])).kind).toBe("unknown");
  });
});

describe("keyboard and wheel", () => {
  it("maps codes to set-1 scancodes", () => {
    expect(scancode("KeyA")).toBe(0x1e);
    expect(scancode("Enter")).toBe(0x1c);
    expect(scancode("NumpadEnter")).toBe(0xe01c);
    expect(scancode("ControlRight")).toBe(0xe01d);
    expect(scancode("MetaLeft")).toBe(0xe05b);
    expect(scancode("Fn")).toBeNull();
  });

  it("turns pixel deltas into notches", () => {
    const w = new WheelAccumulator();
    expect(w.push(30, 0)).toBe(0);
    expect(w.push(30, 0)).toBe(-120); // 60 px down → one notch towards the user
    expect(w.push(-110, 0)).toBe(240); // back up: 10 - 110 = -100 → two notches up
    expect(w.push(3, 1)).toBe(-120); // line mode: a notch per event
  });
});

describe("RdpKeyboard", () => {
  const k = (code: string, mods: Partial<{ metaKey: boolean; ctrlKey: boolean; altKey: boolean }> = {}) => ({ code, metaKey: false, ctrlKey: false, altKey: false, ...mods });
  const keys = (sent: RdpInput[]) => sent.map((e) => (e.t === "key" ? `${e.down ? "+" : "-"}${e.code.toString(16)}` : e.t === "clipboard" ? `clip:${e.text}` : e.t));
  const [CTRL, V, C] = ["1d", "2f", "2e"];

  it("maps ⌘ to Ctrl on macOS and releases keys whose key-up macOS swallows", () => {
    const sent: RdpInput[] = [];
    const kb = new RdpKeyboard("mac", (e) => sent.push(e));
    expect(kb.key(k("MetaLeft", { metaKey: true }), true)).toBe(true);
    kb.key(k("KeyC", { metaKey: true }), true);
    kb.key(k("MetaLeft"), false);
    expect(keys(sent)).toEqual([`+${CTRL}`, `+${C}`, `-${C}`, `-${CTRL}`]);
  });

  it("sends the pasted text before the paste keystroke", () => {
    const sent: RdpInput[] = [];
    const kb = new RdpKeyboard("mac", (e) => sent.push(e));
    kb.key(k("MetaLeft", { metaKey: true }), true);
    // Not taken: the webview's paste runs.
    expect(kb.key(k("KeyV", { metaKey: true }), true)).toBe(false);
    kb.paste("hello");
    expect(kb.key(k("KeyV", { metaKey: true }), false)).toBe(true);
    kb.key(k("MetaLeft"), false);
    expect(keys(sent)).toEqual([`+${CTRL}`, "clip:hello", `+${V}`, `-${V}`, `-${CTRL}`]);
  });

  it("sends the keystroke alone when no paste event comes, and drops a late one", () => {
    vi.useFakeTimers();
    try {
      const sent: RdpInput[] = [];
      const kb = new RdpKeyboard("other", (e) => sent.push(e));
      kb.key(k("ControlLeft", { ctrlKey: true }), true);
      kb.key(k("KeyV", { ctrlKey: true }), true);
      vi.advanceTimersByTime(PASTE_WAIT_MS);
      kb.paste("late");
      expect(keys(sent)).toEqual([`+${CTRL}`, `+${V}`, `-${V}`]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("pastes from the Edit menu with the server's shortcut", () => {
    const sent: RdpInput[] = [];
    new RdpKeyboard("mac", (e) => sent.push(e)).paste("hello");
    expect(keys(sent)).toEqual(["clip:hello", `+${CTRL}`, `+${V}`, `-${V}`, `-${CTRL}`]);
  });

  it("cuts a long paste to what an input batch carries, by encoded size", () => {
    const size = (t: string) => new TextEncoder().encode(JSON.stringify(t)).length;
    expect(fitPaste("short")).toBe("short");
    // 90k CJK characters are ~270 KB of UTF-8: under any character cap, over the byte limit.
    const cjk = fitPaste("漢".repeat(90_000));
    expect(size(cjk)).toBeLessThanOrEqual(MAX_PASTE_BYTES);
    expect(size(cjk + "漢")).toBeGreaterThan(MAX_PASTE_BYTES);
    // Control characters grow sixfold in JSON; a surrogate pair is never split.
    expect(size(fitPaste("\u0001".repeat(60_000)))).toBeLessThanOrEqual(MAX_PASTE_BYTES);
    const emoji = fitPaste("😀".repeat(60_000));
    expect(emoji.length % 2).toBe(0);
    expect(size(emoji)).toBeLessThanOrEqual(MAX_PASTE_BYTES);
  });

  it("leaves the clipboard to the system on Windows", () => {
    const sent: RdpInput[] = [];
    const kb = new RdpKeyboard("windows", (e) => sent.push(e));
    kb.key(k("ControlLeft", { ctrlKey: true }), true);
    expect(kb.key(k("KeyV", { ctrlKey: true }), true)).toBe(true);
    kb.paste("ignored");
    kb.key(k("MetaLeft", { metaKey: true }), true);
    expect(keys(sent)).toEqual([`+${CTRL}`, `+${V}`, "+e05b"]);
  });

  it("knows the platforms", () => {
    expect([keyboardPlatform("MacIntel"), keyboardPlatform("Win32"), keyboardPlatform("Linux x86_64")]).toEqual(["mac", "windows", "other"]);
  });
});
