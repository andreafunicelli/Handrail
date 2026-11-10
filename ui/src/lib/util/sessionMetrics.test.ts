import { describe, expect, it } from "vitest";
import { clockTime, metricLevel, metricsFreshness, secondsSince, showsNumbers, usedPct } from "./sessionMetrics";
import { connectionAddress, hostForAddress, mostSensitiveEnv } from "./sessionContext";

describe("metric thresholds", () => {
  it("reads percentages against 80 / 90", () => {
    expect(metricLevel(null)).toBeNull();
    expect(metricLevel(undefined)).toBeNull();
    expect(metricLevel(Number.NaN)).toBeNull();
    expect(metricLevel(0)).toBe("ok");
    expect(metricLevel(79.9)).toBe("ok");
    expect(metricLevel(80)).toBe("warn");
    expect(metricLevel(89.99)).toBe("warn");
    expect(metricLevel(90)).toBe("danger");
    expect(metricLevel(100)).toBe("danger");
  });

  it("computes percent used only with both numbers and a total", () => {
    expect(usedPct(2, 8)).toBe(25);
    expect(usedPct(null, 8)).toBeNull();
    expect(usedPct(2, null)).toBeNull();
    expect(usedPct(2, 0)).toBeNull();
  });
});

describe("metric freshness", () => {
  const base = { live: true, failed: false, lastOk: 1_000, now: 2_000, pollMs: 5_000 };
  it("is offline whenever the session is disconnected, even with old numbers", () => {
    expect(metricsFreshness({ ...base, live: false })).toBe("offline");
    expect(metricsFreshness({ ...base, live: false, failed: true })).toBe("offline");
  });
  it("is unavailable once polling gave up", () => {
    expect(metricsFreshness({ ...base, failed: true })).toBe("unavailable");
  });
  it("is loading before the first reading", () => {
    expect(metricsFreshness({ ...base, lastOk: null })).toBe("loading");
  });
  it("turns stale after a few missed polls", () => {
    expect(metricsFreshness({ ...base, now: 1_000 + 12_500 })).toBe("live");
    expect(metricsFreshness({ ...base, now: 1_000 + 12_501 })).toBe("stale");
  });
  it("shows numbers only when live or stale", () => {
    expect(showsNumbers("live")).toBe(true);
    expect(showsNumbers("stale")).toBe(true);
    expect(showsNumbers("offline")).toBe(false);
    expect(showsNumbers("unavailable")).toBe(false);
    expect(showsNumbers("loading")).toBe(false);
  });
  it("formats clock time and elapsed seconds", () => {
    const d = new Date(2026, 0, 2, 3, 4, 5).getTime();
    expect(clockTime(d)).toBe("03:04:05");
    expect(secondsSince(d, d + 9_999)).toBe(9);
    expect(secondsSince(d, d - 1_000)).toBe(0);
  });
});

describe("session context", () => {
  it("builds user@host:port addresses", () => {
    expect(connectionAddress({ user: "deploy", host: "10.0.1.11", port: 22 })).toBe("deploy@10.0.1.11:22");
    expect(connectionAddress({ user: "", host: "h", port: 2222 })).toBe("h:2222");
    expect(connectionAddress({ user: "u", host: "::1", port: 22 })).toBe("u@[::1]:22");
    expect(hostForAddress("[::1]")).toBe("[::1]");
  });
  it("picks the most sensitive environment", () => {
    expect(mostSensitiveEnv([])).toBe("none");
    expect(mostSensitiveEnv(["dev", null, "staging"])).toBe("staging");
    expect(mostSensitiveEnv(["staging", "prod", "dev"])).toBe("prod");
  });
});
