/** Pure helpers for the server bar under SSH terminals: thresholds and how fresh the numbers are. */

export type MetricLevel = "ok" | "warn" | "danger";

/** CPU, memory and disk use at or above this percent are shown as a warning… */
export const WARN_PCT = 80;
/** …and at or above this one as a danger. */
export const DANGER_PCT = 90;

/** How a usage percent reads against the thresholds; null when there is no number. */
export function metricLevel(pct: number | null | undefined): MetricLevel | null {
  if (pct == null || !Number.isFinite(pct)) return null;
  if (pct >= DANGER_PCT) return "danger";
  if (pct >= WARN_PCT) return "warn";
  return "ok";
}

/** Percent of `total` that `used` is; null when either is unknown or the total is zero. */
export function usedPct(used: number | null | undefined, total: number | null | undefined): number | null {
  if (used == null || total == null || !total || !Number.isFinite(used) || !Number.isFinite(total)) return null;
  return (used / total) * 100;
}

/**
 * - `offline`: the session is disconnected, nothing is being read.
 * - `unavailable`: the server stopped answering (no /proc, no `sh`): polling gave up.
 * - `loading`: no reading yet.
 * - `stale`: the last good reading is older than a few polls (failed reads, hidden window).
 * - `live`: the numbers are current.
 */
export type Freshness = "offline" | "unavailable" | "loading" | "stale" | "live";

export interface FreshnessInput {
  /** The session is connected. */
  live: boolean;
  /** Polling gave up after repeated failures. */
  failed: boolean;
  /** When the last good reading arrived (ms epoch), null before the first. */
  lastOk: number | null;
  now: number;
  pollMs: number;
}

/** Readings older than this many poll intervals are stale. */
export const STALE_POLLS = 2.5;

export function metricsFreshness({ live, failed, lastOk, now, pollMs }: FreshnessInput): Freshness {
  if (!live) return "offline";
  if (failed) return "unavailable";
  if (lastOk == null) return "loading";
  return now - lastOk > pollMs * STALE_POLLS ? "stale" : "live";
}

/** Whether numbers may be shown at all (dimmed when stale); never for an offline session. */
export const showsNumbers = (f: Freshness) => f === "live" || f === "stale";

/** "14:05:09": a 24-hour local clock time. */
export function clockTime(ms: number): string {
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

/** Whole seconds between `then` and `now`, never negative. */
export const secondsSince = (then: number, now: number) => Math.max(0, Math.floor((now - then) / 1000));
