import type { Environment } from "../api/types";

/** `host`, bracketed when it is an IPv6 address (so a port can follow it). */
export function hostForAddress(host: string): string {
  return host.includes(":") && !host.startsWith("[") ? `[${host}]` : host;
}

/** `user@host:port` (the user left out when empty). */
export function connectionAddress(c: { user?: string | null; host: string; port: number }): string {
  const u = c.user ? `${c.user}@` : "";
  return `${u}${hostForAddress(c.host)}:${c.port}`;
}

const RANK: Record<Environment, number> = { prod: 3, staging: 2, dev: 1, none: 0 };

/** The most sensitive of `envs` (production first), "none" when empty. */
export function mostSensitiveEnv(envs: Iterable<Environment | null | undefined>): Environment {
  let best: Environment = "none";
  for (const e of envs) if (e && RANK[e] > RANK[best]) best = e;
  return best;
}
