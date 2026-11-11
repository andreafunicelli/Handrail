import type { Connection, ConnectionKind, Environment } from "../api/types";
import { fuzzyScore } from "./fuzzy";

/** A filter word of the connection search: `env:prod`, `-type:rdp`, `tag:"web tier"`. */
export interface QueryToken {
  key: FilterKey;
  value: string;
  /** Excluded (`-env:prod`). */
  not: boolean;
  /** As typed, to remove it from the query. */
  raw: string;
}

export type FilterKey = "env" | "type" | "tag" | "host" | "group" | "is" | "user";

/** A parsed search: filter tokens plus free text (fuzzy over name, host, user and tags). */
export interface ConnQuery {
  tokens: QueryToken[];
  text: string;
}

const KEYS: Record<string, FilterKey> = {
  env: "env",
  environment: "env",
  type: "type",
  kind: "type",
  tag: "tag",
  tags: "tag",
  host: "host",
  group: "group",
  folder: "group",
  is: "is",
  user: "user",
};

const ENV_ALIASES: Record<string, Environment> = {
  prod: "prod",
  production: "prod",
  prd: "prod",
  staging: "staging",
  stage: "staging",
  stg: "staging",
  dev: "dev",
  development: "dev",
  none: "none",
};

const TYPE_ALIASES: Record<string, ConnectionKind[]> = {
  ssh: ["ssh"],
  sftp: ["sftp"],
  rdp: ["rdp"],
  local: ["local"],
  mysql: ["mysql"],
  mariadb: ["mysql"],
  postgres: ["postgres"],
  postgresql: ["postgres"],
  pg: ["postgres"],
  db: ["mysql", "postgres"],
  database: ["mysql", "postgres"],
};

/** Split `q` into filter tokens and free text. Unknown `key:` words stay text. */
export function parseQuery(q: string): ConnQuery {
  const tokens: QueryToken[] = [];
  const text: string[] = [];
  const re = /(-?)([a-z]+):(?:"([^"]*)"?|(\S*))|"([^"]*)"?|(\S+)/gi;
  for (const m of q.matchAll(re)) {
    const raw = m[0];
    if (m[2] !== undefined) {
      const key = KEYS[m[2].toLowerCase()];
      const value = (m[3] ?? m[4] ?? "").trim();
      if (key) {
        // `env:prod,staging` is two tokens of the same key (either matches).
        for (const v of value.split(",")) tokens.push({ key, value: v.trim().toLowerCase(), not: m[1] === "-", raw });
        continue;
      }
    }
    text.push(m[5] ?? raw);
  }
  return { tokens, text: text.join(" ").trim() };
}

/** `q` without the token typed as `raw` (a filter chip's ×). */
export function removeToken(q: string, raw: string): string {
  const i = q.indexOf(raw);
  if (i < 0) return q;
  return (q.slice(0, i) + q.slice(i + raw.length)).replace(/\s{2,}/g, " ").trim();
}

/** What a connection is matched against besides its own fields. */
export interface QueryContext {
  /** The folder path of a connection ("Production / Web"). */
  groupPath: (c: Connection) => string;
  /** Connections with an open session. */
  active: ReadonlySet<string>;
  pinned: ReadonlySet<string>;
}

function tokenMatches(c: Connection, t: QueryToken, ctx: QueryContext): boolean {
  const v = t.value;
  if (!v) return true;
  switch (t.key) {
    case "env": {
      const env = ENV_ALIASES[v];
      return env ? c.env === env : c.env.startsWith(v);
    }
    case "type": {
      const kinds = TYPE_ALIASES[v];
      return kinds ? kinds.includes(c.kind) : c.kind.startsWith(v);
    }
    case "tag":
      return c.tags.some((tag) => tag.toLowerCase().includes(v));
    case "host":
      return c.host.toLowerCase().includes(v);
    case "user":
      return c.user.toLowerCase().includes(v);
    case "group":
      return ctx.groupPath(c).toLowerCase().includes(v);
    case "is":
      if (v === "active" || v === "open" || v === "connected") return ctx.active.has(c.id);
      if (v === "fav" || v === "favorite" || v === "favourite" || v === "pinned") return ctx.pinned.has(c.id);
      return false;
  }
}

/** Whether `c` passes the query's filters (same key: any value; different keys: all). The
 *  free text is scored separately (`textScore`). */
export function passesFilters(c: Connection, q: ConnQuery, ctx: QueryContext): boolean {
  const byKey = new Map<string, QueryToken[]>();
  for (const t of q.tokens) {
    const k = `${t.not ? "-" : ""}${t.key}`;
    const list = byKey.get(k);
    if (list) list.push(t);
    else byKey.set(k, [t]);
  }
  for (const [k, list] of byKey) {
    const any = list.some((t) => tokenMatches(c, t, ctx));
    if (k.startsWith("-") ? any : !any) return false;
  }
  return true;
}

/** Fuzzy score of the free text against a connection (null: no match; 0 with no text). */
export function textScore(c: Connection, text: string): number | null {
  if (!text) return 0;
  let best: number | null = null;
  for (const f of [c.name, c.host, c.user ? `${c.user}@${c.host}` : "", ...c.tags]) {
    if (!f) continue;
    const s = fuzzyScore(text, f);
    if (s != null && (best == null || s > best)) best = s;
  }
  return best;
}

/** The connections matching `q`, best text matches first (input order when there is no text). */
export function filterConnections(list: Connection[], q: ConnQuery, ctx: QueryContext): Connection[] {
  const out: { c: Connection; s: number; i: number }[] = [];
  list.forEach((c, i) => {
    if (!passesFilters(c, q, ctx)) return;
    const s = textScore(c, q.text);
    if (s != null) out.push({ c, s, i });
  });
  if (q.text) out.sort((a, b) => b.s - a.s || a.i - b.i);
  return out.map((x) => x.c);
}

/** Suggestions for the filter syntax, for the search box's hint. */
export const FILTER_EXAMPLES = ["env:prod", "type:ssh", "tag:web", "host:10.200", "group:production", "is:active", "is:fav"];
