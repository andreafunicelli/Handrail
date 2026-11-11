import { describe, expect, it } from "vitest";
import type { Connection } from "../api/types";
import { filterConnections, parseQuery, removeToken, type QueryContext } from "./connQuery";

const conn = (id: string, over: Partial<Connection>): Connection => ({
  id,
  name: id,
  kind: "ssh",
  group_id: null,
  host: "10.0.0.1",
  port: 22,
  user: "root",
  auth: { type: "agent" } as Connection["auth"],
  jump: [],
  env: "none",
  tags: [],
  color: null,
  approval_mode: null,
  local_shell: null,
  notes: "",
  tunnels: [],
  ...over,
});

const list = [
  conn("web-01", { env: "prod", host: "10.200.1.10", tags: ["web", "nginx"] }),
  conn("web-02", { env: "staging", host: "10.200.1.11", tags: ["web"] }),
  conn("db-main", { kind: "mysql", env: "prod", host: "db.internal", tags: ["mysql"] }),
  conn("win-jump", { kind: "rdp", env: "dev", host: "192.168.1.5" }),
  conn("analytics", { kind: "postgres", env: "none", host: "pg.internal" }),
];
const groups: Record<string, string> = { "web-01": "Production / Web", "db-main": "Production / Data", "web-02": "Staging" };
const ctx: QueryContext = {
  groupPath: (c) => groups[c.id] ?? "",
  active: new Set(["web-02", "win-jump"]),
  pinned: new Set(["db-main"]),
};
const ids = (q: string) => filterConnections(list, parseQuery(q), ctx).map((c) => c.id);

describe("connection query", () => {
  it("parses filters, aliases, negation, lists and quoted values", () => {
    const q = parseQuery('env:production -type:rdp tag:"web tier" web host:10.200 tag:a,b foo:bar');
    expect(q.text).toBe("web foo:bar");
    expect(q.tokens.map((t) => `${t.not ? "-" : ""}${t.key}=${t.value}`)).toEqual([
      "env=production",
      "-type=rdp",
      "tag=web tier",
      "host=10.200",
      "tag=a",
      "tag=b",
    ]);
  });

  it("combines filters: same key any, different keys all", () => {
    expect(ids("env:prod")).toEqual(["web-01", "db-main"]);
    expect(ids("env:prod type:ssh")).toEqual(["web-01"]);
    expect(ids("env:prod,staging")).toEqual(["web-01", "web-02", "db-main"]);
    expect(ids("type:db")).toEqual(["db-main", "analytics"]);
    expect(ids("type:pg")).toEqual(["analytics"]);
    expect(ids("tag:mysql")).toEqual(["db-main"]);
    expect(ids("host:10.200")).toEqual(["web-01", "web-02"]);
    expect(ids("group:production")).toEqual(["web-01", "db-main"]);
    expect(ids("-env:prod -type:rdp")).toEqual(["web-02", "analytics"]);
    expect(ids("is:active")).toEqual(["web-02", "win-jump"]);
    expect(ids("is:fav")).toEqual(["db-main"]);
    expect(ids("env:dev type:rdp is:active")).toEqual(["win-jump"]);
  });

  it("mixes filters with fuzzy text, best match first", () => {
    expect(ids("env:prod web")).toEqual(["web-01"]);
    expect(ids("w02")).toEqual(["web-02"]);
    expect(ids("nginx")).toEqual(["web-01"]);
    expect(ids("type:ssh zzz")).toEqual([]);
  });

  it("removes a chip's token", () => {
    expect(removeToken("env:prod  web type:ssh", "env:prod")).toBe("web type:ssh");
    expect(removeToken("a b", "c")).toBe("a b");
  });

  it("stays fast on a large inventory", () => {
    const many = Array.from({ length: 2000 }, (_, i) => conn(`srv-${i}`, { env: i % 3 ? "dev" : "prod", host: `10.${i % 250}.0.${i % 200}`, tags: [`t${i % 17}`] }));
    const t0 = performance.now();
    for (let k = 0; k < 20; k++) filterConnections(many, parseQuery("env:prod tag:t3 srv"), ctx);
    expect((performance.now() - t0) / 20).toBeLessThan(25);
  });
});
