import type { Tunnel, TunnelStatus } from "../api/types";

const LOOPBACK = new Set(["127.0.0.1", "localhost", "::1"]);
const host = (h: string) => (LOOPBACK.has(h) ? "localhost" : h);

/** Short form for the session bar: "15432 → db:5432", "SOCKS 1080", "server 8080 → 3000". */
export function tunnelLabel(x: TunnelStatus): string {
  const t = x.tunnel;
  const port = x.port ?? t.bind_port;
  const dest = `${host(t.dest_host)}:${t.dest_port}`;
  if (t.kind === "dynamic") return `SOCKS ${port || "…"}`;
  if (t.kind === "remote") return `⇠ ${port || "…"} → ${dest}`;
  return `${port || "…"} → ${dest}`;
}

/** What to connect to on this computer, for a running local or dynamic forward. */
export function tunnelAddress(x: TunnelStatus): string | null {
  if (x.port == null || x.tunnel.kind === "remote") return null;
  const h = x.tunnel.bind_host;
  return `${h === "0.0.0.0" || h === "::" ? "localhost" : host(h)}:${x.port}`;
}

export const newTunnel = (): Tunnel => ({ kind: "local", bind_host: "127.0.0.1", bind_port: 0, dest_host: "127.0.0.1", dest_port: 0, enabled: true });

const port = (n: number, zeroOk: boolean) => Number.isInteger(n) && n <= 65535 && (n > 0 || (zeroOk && n === 0));

/** Why a tunnel cannot be saved, or null. Port 0 to listen on means "any free port". */
export function tunnelProblem(t: Tunnel): "bind" | "dest" | null {
  if (!t.bind_host.trim() || !port(t.bind_port, true)) return "bind";
  if (t.kind !== "dynamic" && (!t.dest_host.trim() || !port(t.dest_port, false))) return "dest";
  return null;
}
