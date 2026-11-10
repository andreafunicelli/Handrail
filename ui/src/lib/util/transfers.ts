/** Transfer records and the views of them the transfer panel shows (pure: no stores). */

export interface Transfer {
  id: string;
  /** `FileBackend.key` of the browser that started it. */
  backend: string;
  /** A copy's destination browser (`FileBackend.key`): it lists the transfer too. */
  to?: string;
  direction: "up" | "down" | "copy";
  name: string;
  /** Where it comes from, for people: "web-01: /var/log/syslog", a local path. */
  source: string;
  /** Where it goes, for people: "backups (S3): /bucket/dir", "Downloads". */
  dest: string;
  done: number;
  total: number;
  state: "running" | "done" | "failed";
  /** Local path (downloads) or remote path (uploads, copies) when done; the error when failed. */
  result: string | null;
  /** When it started and ended (ms since the epoch). */
  started: number;
  ended?: number;
}

/** Whole percent done, 0–100; null while the size is not known yet. */
export function percent(t: Pick<Transfer, "done" | "total" | "state">): number | null {
  if (t.state === "done") return 100;
  if (!t.total) return null;
  return Math.min(100, Math.floor((t.done / t.total) * 100));
}

export const runningCount = (list: readonly Transfer[]) => list.reduce((n, t) => n + (t.state === "running" ? 1 : 0), 0);
export const failedCount = (list: readonly Transfer[]) => list.reduce((n, t) => n + (t.state === "failed" ? 1 : 0), 0);

/** Running first (oldest first: they finish first), then the finished ones, newest first. */
export function ordered(list: readonly Transfer[]): Transfer[] {
  const running = list.filter((t) => t.state === "running").sort((a, b) => a.started - b.started);
  const finished = list.filter((t) => t.state !== "running").sort((a, b) => (b.ended ?? b.started) - (a.ended ?? a.started));
  return [...running, ...finished];
}

/**
 * The record once its operation settles: `done` only when it resolved; a rejection (an error,
 * a lost connection) is always `failed`, with the error, never shown as completed.
 */
export function settle(t: Transfer, outcome: { ok: true; value: string } | { ok: false; error: string }, now = Date.now()): Transfer {
  if (outcome.ok) return { ...t, state: "done", result: outcome.value, done: Math.max(t.done, t.total), ended: now };
  return { ...t, state: "failed", result: outcome.error, ended: now };
}

/** Progress events only move a running transfer forward. */
export function progressed(t: Transfer, done: number, total: number): Transfer {
  if (t.state !== "running") return t;
  return { ...t, done: Math.max(0, done), total: Math.max(0, total) };
}

/** Keeps the running ones and the `keep` most recent finished ones (the list lives in memory). */
export function trimmed(list: readonly Transfer[], keep = 200): Transfer[] {
  const finished = list.filter((t) => t.state !== "running");
  if (finished.length <= keep) return [...list];
  const drop = new Set(
    finished
      .sort((a, b) => (a.ended ?? a.started) - (b.ended ?? b.started))
      .slice(0, finished.length - keep)
      .map((t) => t.id),
  );
  return list.filter((t) => !drop.has(t.id));
}
