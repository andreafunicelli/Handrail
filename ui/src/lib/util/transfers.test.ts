import { describe, expect, it } from "vitest";
import { failedCount, ordered, percent, progressed, runningCount, settle, trimmed, type Transfer } from "./transfers";
import { transfers } from "../stores/transfers.svelte";
import { sftpBackend, type FileBackend } from "./fileBackends";

const rec = (id: string, over: Partial<Transfer> = {}): Transfer => ({
  id,
  backend: "sftp:a",
  direction: "down",
  name: id,
  source: "a: /x",
  dest: "Downloads",
  done: 0,
  total: 0,
  state: "running",
  result: null,
  started: 0,
  ...over,
});

describe("transfer selectors", () => {
  it("percent: unknown until the size is known, 100 only when done", () => {
    expect(percent(rec("a"))).toBeNull();
    expect(percent(rec("a", { done: 50, total: 200 }))).toBe(25);
    expect(percent(rec("a", { done: 200, total: 200 }))).toBe(100);
    expect(percent(rec("a", { done: 300, total: 200 }))).toBe(100);
    expect(percent(rec("a", { state: "done" }))).toBe(100);
    // A failed transfer keeps where it stopped.
    expect(percent(rec("a", { state: "failed", done: 10, total: 100 }))).toBe(10);
  });

  it("counts and order: running first (oldest first), then newest finished", () => {
    const list = [
      rec("r2", { started: 5 }),
      rec("d1", { state: "done", started: 1, ended: 2 }),
      rec("f1", { state: "failed", started: 3, ended: 9 }),
      rec("r1", { started: 4 }),
    ];
    expect(runningCount(list)).toBe(2);
    expect(failedCount(list)).toBe(1);
    expect(ordered(list).map((t) => t.id)).toEqual(["r1", "r2", "f1", "d1"]);
  });

  it("settle: a rejection is failed with its error, never done", () => {
    const t = rec("a", { done: 40, total: 100 });
    const failed = settle(t, { ok: false, error: "connection lost" }, 7);
    expect(failed).toMatchObject({ state: "failed", result: "connection lost", done: 40, ended: 7 });
    const done = settle(t, { ok: true, value: "/dl/a" }, 8);
    expect(done).toMatchObject({ state: "done", result: "/dl/a", done: 100 });
  });

  it("progress only moves running transfers", () => {
    expect(progressed(rec("a"), 5, 10)).toMatchObject({ done: 5, total: 10 });
    const failed = rec("a", { state: "failed", done: 3, total: 10 });
    expect(progressed(failed, 10, 10)).toBe(failed);
  });

  it("trimmed keeps every running one and the newest finished ones", () => {
    const list = [rec("r"), ...Array.from({ length: 5 }, (_, i) => rec(`d${i}`, { state: "done", ended: i }))];
    expect(trimmed(list, 2).map((t) => t.id)).toEqual(["r", "d3", "d4"]);
  });
});

describe("transfers store", () => {
  const fake = (download: FileBackend["download"]): FileBackend => ({ ...sftpBackend("fake"), download });

  it("records a failed download as failed, with the error", async () => {
    const fs = fake(() => Promise.reject(new Error("connection lost")));
    await expect(transfers.download(fs, "/var/log/syslog")).rejects.toThrow();
    const t = transfers.list.find((x) => x.name === "syslog")!;
    expect(t.state).toBe("failed");
    expect(t.result).toContain("connection lost");
    expect(t.source).toContain("/var/log/syslog");
    expect(transfers.running).toBe(0);
  });

  it("counts a running transfer and marks it done when it resolves", async () => {
    let finish: (v: string) => void = () => {};
    const fs = fake(() => new Promise((r) => (finish = r)));
    const p = transfers.download(fs, "/srv/a.txt");
    await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
    expect(transfers.running).toBe(1);
    finish("/Downloads/a.txt");
    await p;
    const t = transfers.list.find((x) => x.name === "a.txt")!;
    expect(t.state).toBe("done");
    expect(transfers.running).toBe(0);
    transfers.clearFinished();
    expect(transfers.list.length).toBe(0);
  });
});
