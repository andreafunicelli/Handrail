/**
 * The RDP helper's messages as the UI receives them (`[kind][payload]`, see
 * rdp-helper/src/proto.rs), and browser keys → PC/AT set-1 scancodes.
 */

import type { RdpInput } from "../api/types";

export const MSG_TILES = 0x11;
export const MSG_POINTER = 0x12;
export const MSG_ERROR = 0x13;
export const MSG_TERMINATED = 0x14;

export interface TileRect {
  x: number;
  y: number;
  w: number;
  h: number;
  /** RGBA, row by row. */
  data: Uint8ClampedArray<ArrayBuffer>;
}

export interface Tiles {
  width: number;
  height: number;
  rects: TileRect[];
}

export type Pointer = { kind: "default" } | { kind: "hidden" } | { kind: "bitmap"; width: number; height: number; hotX: number; hotY: number; rgba: Uint8ClampedArray<ArrayBuffer> };

export type RdpMessage =
  | { kind: "tiles"; tiles: Tiles }
  | { kind: "pointer"; pointer: Pointer }
  | { kind: "error"; message: string }
  | { kind: "terminated"; reason: string; error: boolean }
  | { kind: "unknown" };

/** Decode one message; the payload starts at byte 1. */
export function parseMessage(buf: ArrayBuffer): RdpMessage {
  const bytes = new Uint8Array(buf);
  const view = new DataView(buf);
  const u16 = (i: number) => view.getUint16(i, true);
  const json = () => JSON.parse(new TextDecoder().decode(bytes.subarray(1))) as Record<string, unknown>;
  switch (bytes[0]) {
    case MSG_TILES: {
      const width = u16(1);
      const height = u16(3);
      const count = u16(5);
      let at = 7;
      const rects: TileRect[] = [];
      for (let i = 0; i < count; i++) {
        const [x, y, w, h] = [u16(at), u16(at + 2), u16(at + 4), u16(at + 6)];
        at += 8;
        const len = w * h * 4;
        if (at + len > bytes.length) throw new Error("truncated tiles");
        rects.push({ x, y, w, h, data: new Uint8ClampedArray(buf, at, len) as Uint8ClampedArray<ArrayBuffer> });
        at += len;
      }
      return { kind: "tiles", tiles: { width, height, rects } };
    }
    case MSG_POINTER: {
      if (bytes[1] === 0) return { kind: "pointer", pointer: { kind: "default" } };
      if (bytes[1] === 1) return { kind: "pointer", pointer: { kind: "hidden" } };
      if (bytes[1] === 3) {
        const [width, height, hotX, hotY] = [u16(2), u16(4), u16(6), u16(8)];
        const rgba = new Uint8ClampedArray(buf, 10, width * height * 4) as Uint8ClampedArray<ArrayBuffer>;
        return { kind: "pointer", pointer: { kind: "bitmap", width, height, hotX, hotY, rgba } };
      }
      return { kind: "unknown" };
    }
    case MSG_ERROR:
      return { kind: "error", message: String(json().message ?? "") };
    case MSG_TERMINATED: {
      const j = json();
      return { kind: "terminated", reason: String(j.reason ?? ""), error: !!j.error };
    }
    default:
      return { kind: "unknown" };
  }
}

/** The inverse of the tiles decoding, for the in-browser mock. `rgba(x, y)` gives a pixel. */
export function encodeTiles(width: number, height: number, rects: { x: number; y: number; w: number; h: number }[], rgba: (x: number, y: number) => [number, number, number]): ArrayBuffer {
  const size = 7 + rects.reduce((n, r) => n + 8 + r.w * r.h * 4, 0);
  const buf = new ArrayBuffer(size);
  const view = new DataView(buf);
  const bytes = new Uint8Array(buf);
  bytes[0] = MSG_TILES;
  view.setUint16(1, width, true);
  view.setUint16(3, height, true);
  view.setUint16(5, rects.length, true);
  let at = 7;
  for (const r of rects) {
    [r.x, r.y, r.w, r.h].forEach((v, i) => view.setUint16(at + i * 2, v, true));
    at += 8;
    for (let y = r.y; y < r.y + r.h; y++)
      for (let x = r.x; x < r.x + r.w; x++) {
        const [red, green, blue] = rgba(x, y);
        bytes.set([red, green, blue, 255], at);
        at += 4;
      }
  }
  return buf;
}

/** Extended keys carry 0xE0 in the high byte. */
const SCANCODES: Record<string, number> = {
  Escape: 0x01, Digit1: 0x02, Digit2: 0x03, Digit3: 0x04, Digit4: 0x05, Digit5: 0x06, Digit6: 0x07, Digit7: 0x08,
  Digit8: 0x09, Digit9: 0x0a, Digit0: 0x0b, Minus: 0x0c, Equal: 0x0d, Backspace: 0x0e, Tab: 0x0f,
  KeyQ: 0x10, KeyW: 0x11, KeyE: 0x12, KeyR: 0x13, KeyT: 0x14, KeyY: 0x15, KeyU: 0x16, KeyI: 0x17, KeyO: 0x18, KeyP: 0x19,
  BracketLeft: 0x1a, BracketRight: 0x1b, Enter: 0x1c, ControlLeft: 0x1d,
  KeyA: 0x1e, KeyS: 0x1f, KeyD: 0x20, KeyF: 0x21, KeyG: 0x22, KeyH: 0x23, KeyJ: 0x24, KeyK: 0x25, KeyL: 0x26,
  Semicolon: 0x27, Quote: 0x28, Backquote: 0x29, ShiftLeft: 0x2a, Backslash: 0x2b,
  KeyZ: 0x2c, KeyX: 0x2d, KeyC: 0x2e, KeyV: 0x2f, KeyB: 0x30, KeyN: 0x31, KeyM: 0x32,
  Comma: 0x33, Period: 0x34, Slash: 0x35, ShiftRight: 0x36, NumpadMultiply: 0x37, AltLeft: 0x38, Space: 0x39, CapsLock: 0x3a,
  F1: 0x3b, F2: 0x3c, F3: 0x3d, F4: 0x3e, F5: 0x3f, F6: 0x40, F7: 0x41, F8: 0x42, F9: 0x43, F10: 0x44,
  NumLock: 0x45, ScrollLock: 0x46, Numpad7: 0x47, Numpad8: 0x48, Numpad9: 0x49, NumpadSubtract: 0x4a,
  Numpad4: 0x4b, Numpad5: 0x4c, Numpad6: 0x4d, NumpadAdd: 0x4e, Numpad1: 0x4f, Numpad2: 0x50, Numpad3: 0x51,
  Numpad0: 0x52, NumpadDecimal: 0x53, IntlBackslash: 0x56, F11: 0x57, F12: 0x58,
  KanaMode: 0x70, IntlRo: 0x73, Convert: 0x79, NonConvert: 0x7b, IntlYen: 0x7d,
  NumpadEnter: 0xe01c, ControlRight: 0xe01d, NumpadDivide: 0xe035, PrintScreen: 0xe037, AltRight: 0xe038,
  Home: 0xe047, ArrowUp: 0xe048, PageUp: 0xe049, ArrowLeft: 0xe04b, ArrowRight: 0xe04d, End: 0xe04f,
  ArrowDown: 0xe050, PageDown: 0xe051, Insert: 0xe052, Delete: 0xe053,
  MetaLeft: 0xe05b, MetaRight: 0xe05c, ContextMenu: 0xe05d,
};

/** The scancode for a `KeyboardEvent.code`, or null for keys RDP has no code for here. */
export function scancode(code: string): number | null {
  return SCANCODES[code] ?? null;
}

/** Wheel input: pixel deltas add up to notches of 120 (one per 50 px); line or page deltas are a notch each. */
export class WheelAccumulator {
  private acc = 0;
  /** Units to send for this event (positive = away from the user, i.e. scroll up); 0 for none yet. */
  push(delta: number, mode: number): number {
    if (mode !== 0) return delta === 0 ? 0 : delta > 0 ? -120 : 120;
    this.acc += delta;
    const notches = Math.trunc(this.acc / 50);
    if (!notches) return 0;
    this.acc -= notches * 50;
    return -notches * 120;
  }
}

export type KeyboardPlatform = "mac" | "windows" | "other";

export function keyboardPlatform(platform: string): KeyboardPlatform {
  return /Mac|iP/.test(platform) ? "mac" : /Win/.test(platform) ? "windows" : "other";
}

/** Longest wait for the webview's paste event after the paste keystroke. */
export const PASTE_WAIT_MS = 300;
/** Longer pastes are cut: the helper refuses input messages over 1 MiB. */
export const MAX_PASTE = 100_000;

type KeyLike = Pick<KeyboardEvent, "code" | "metaKey" | "ctrlKey" | "altKey">;

/**
 * Keyboard → RDP input. On macOS, ⌘ acts as Ctrl (⌘C copies on the server, as in Microsoft's
 * client). Outside Windows, where the helper shares no system clipboard, the paste keystroke
 * (⌘V, Ctrl+V) is held back until the webview's paste event brings the local clipboard's
 * text: that text goes to the server first, then the keystroke, so the server pastes it.
 */
export class RdpKeyboard {
  private pending: ReturnType<typeof setTimeout> | null = null;
  /** The V of a paste was sent whole: its own key-up is dropped. */
  private swallowUp = false;
  /** macOS sends no key-up for keys released while ⌘ is down: released with ⌘ instead. */
  private withMeta = new Set<number>();
  /** A paste keystroke went out without its text until then: a paste event arriving late is
   *  dropped rather than pasting twice. */
  private lateUntil = 0;

  constructor(
    private platform: KeyboardPlatform,
    private send: (e: RdpInput) => void,
  ) {}

  /** True when the key went to the server (the caller prevents its default action). */
  key(e: KeyLike, down: boolean): boolean {
    const mac = this.platform === "mac";
    if (this.platform !== "windows" && e.code === "KeyV" && (mac ? e.metaKey : e.ctrlKey) && !e.altKey) {
      if (down) {
        // The default action is the webview's paste, which calls `paste`.
        this.pending ??= setTimeout(() => this.finish(null), PASTE_WAIT_MS);
        return false;
      }
      if (this.swallowUp) {
        this.swallowUp = false;
        return true;
      }
    }
    if (this.pending) this.finish(null);
    const meta = e.code === "MetaLeft" || e.code === "MetaRight";
    const sc = scancode(mac && meta ? e.code.replace("Meta", "Control") : e.code);
    if (sc == null) return false;
    if (mac) {
      if (meta && !down) {
        for (const k of this.withMeta) this.send({ t: "key", code: k, down: false });
        this.withMeta.clear();
        this.swallowUp = false;
      } else if (!meta) {
        if (down && e.metaKey) this.withMeta.add(sc);
        else if (!down) this.withMeta.delete(sc);
      }
    }
    this.send({ t: "key", code: sc, down });
    return true;
  }

  /** The webview's paste event, with the local clipboard's text (null when it holds none). */
  paste(text: string | null) {
    if (this.pending) return this.finish(text);
    if (Date.now() < this.lateUntil) return;
    // Paste from the Edit menu, without the keystroke: send the server's paste shortcut too.
    if (this.platform === "windows" || !text) return;
    this.offer(text);
    const [ctrl, v] = [scancode("ControlLeft")!, scancode("KeyV")!];
    for (const [code, down] of [[ctrl, true], [v, true], [v, false], [ctrl, false]] as const) this.send({ t: "key", code, down });
  }

  /** Focus lost: nothing is pending any more. */
  reset() {
    if (this.pending) clearTimeout(this.pending);
    this.pending = null;
    this.swallowUp = false;
    this.withMeta.clear();
  }

  private finish(text: string | null) {
    if (this.pending) clearTimeout(this.pending);
    this.pending = null;
    if (text) this.offer(text);
    else this.lateUntil = Date.now() + PASTE_WAIT_MS;
    const v = scancode("KeyV")!;
    this.send({ t: "key", code: v, down: true });
    this.send({ t: "key", code: v, down: false });
    this.swallowUp = true;
  }

  private offer(text: string) {
    this.send({ t: "clipboard", text: text.slice(0, MAX_PASTE) });
  }
}
