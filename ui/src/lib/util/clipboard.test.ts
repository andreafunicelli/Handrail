import { afterEach, describe, expect, it, vi } from "vitest";
import { clearClipboardLater } from "./clipboard";

describe("clearClipboardLater", () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  function stubClipboard(initial: string) {
    const clip = { value: initial, readText: async () => clip.value, writeText: async (v: string) => void (clip.value = v) };
    vi.stubGlobal("navigator", { clipboard: clip });
    return clip;
  }

  it("empties the clipboard once the time is up if it still holds the secret", async () => {
    vi.useFakeTimers();
    const clip = stubClipboard("SECRET");
    clearClipboardLater("SECRET", 60_000);
    await vi.advanceTimersByTimeAsync(59_999);
    expect(clip.value).toBe("SECRET");
    await vi.advanceTimersByTimeAsync(1);
    expect(clip.value).toBe("");
  });

  it("leaves something copied since alone", async () => {
    vi.useFakeTimers();
    const clip = stubClipboard("SECRET");
    clearClipboardLater("SECRET", 1000);
    clip.value = "other";
    await vi.advanceTimersByTimeAsync(1000);
    expect(clip.value).toBe("other");
  });
});
