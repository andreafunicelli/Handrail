import { describe, expect, it, vi } from "vitest";
import { api } from "../api";
import { opsLog } from "./opsLog.svelte";

const input = {
  sessionId: "s1",
  tool: "run_command",
  input: { command: "df -h" },
  output: null,
  isError: false,
  aborted: false,
  actor: "Agent",
  approval: null,
  // Recent, or the retention would drop them before anything is compared.
  startedAt: Date.now() - 2000,
  endedAt: Date.now() - 1500,
};

describe("operations history", () => {
  it("Clear history waits for a write under way, so the cleared records do not come back", async () => {
    // The vault: one old record already stored.
    let stored: unknown = [{ ...input, at: Date.now() - 60_000, session_id: "s1", tool: "run_command", outcome: "ok" }];
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    const get = vi.spyOn(api, "getSetting").mockImplementation(async (key: string) => {
      if (key.startsWith("opslog.") && key !== "opslog.settings") {
        const read = stored; // what the vault held when the read started
        await gate; // and the answer is slow to come back
        return read;
      }
      return null;
    });
    const set = vi.spyOn(api, "setSetting").mockImplementation(async (key: string, value: unknown) => {
      if (key !== "opslog.settings") stored = value;
    });

    opsLog.record(input);
    const flushing = opsLog.flush(); // reads the old history, then waits on the gate
    const clearing = opsLog.clear(); // the user clicks Clear history meanwhile
    release();
    await Promise.all([flushing, clearing]);

    expect(stored).toEqual([]);
    expect(opsLog.stored).toEqual([]);
    expect(opsLog.pending).toEqual([]);
    get.mockRestore();
    set.mockRestore();
  });
});
