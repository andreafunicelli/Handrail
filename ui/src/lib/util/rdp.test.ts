import { describe, expect, it } from "vitest";
import { MSG_ERROR, MSG_POINTER, MSG_TERMINATED, encodeTiles, parseMessage, scancode, WheelAccumulator } from "./rdp";

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
