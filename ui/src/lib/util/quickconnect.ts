export interface QuickTarget {
  user: string | null;
  host: string;
  port: number;
}

/**
 * Parse `user@host[:port]`, `host`, `host:port`, `user@[::1]:22` or `ssh user@host -p 2222`.
 * Returns null for anything that is not a plausible target.
 */
export function parseQuickConnect(input: string): QuickTarget | null {
  let s = input.trim();
  if (!s) return null;
  let port: number | null = null;
  if (/^ssh\s+/i.test(s)) {
    s = s.replace(/^ssh\s+/i, "");
    const p = s.match(/\s-p\s*(\d+)/);
    if (p) {
      port = Number(p[1]);
      s = s.replace(p[0], "");
    }
    s = s.trim();
  }
  if (/\s/.test(s)) return null;
  let user: string | null = null;
  const at = s.lastIndexOf("@");
  if (at !== -1) {
    user = s.slice(0, at);
    s = s.slice(at + 1);
    if (!user || !/^[A-Za-z0-9._\-$\]+$/.test(user)) return null;
  }
  let host = s;
  const v6 = s.match(/^\[([0-9a-fA-F:.]+)\](?::(\d+))?$/);
  if (v6) {
    host = v6[1];
    if (v6[2]) port = Number(v6[2]);
  } else if (s.includes(":")) {
    const m = s.match(/^([^:]+):(\d+)$/);
    if (!m) return null;
    host = m[1];
    port = Number(m[2]);
  }
  if (!v6 && !/^[A-Za-z0-9.\-_]+$/.test(host)) return null;
  if (!host || host.startsWith(".") || host.startsWith("-")) return null;
  const finalPort = port ?? 22;
  if (!Number.isInteger(finalPort) || finalPort < 1 || finalPort > 65535) return null;
  return { user, host, port: finalPort };
}
