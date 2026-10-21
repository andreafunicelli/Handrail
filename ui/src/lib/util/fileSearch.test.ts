import { describe, expect, it } from "vitest";
import type { FileEntry } from "../api/types";
import { nameMatcher, searchTree, type Lister } from "./fileSearch";

const entry = (path: string, kind: FileEntry["kind"] = "file", link = false): FileEntry => ({
  name: path.split("/").pop()!,
  path,
  kind,
  link,
  size: 1,
  modified: null,
  permissions: null,
});

/** A tree as a map of folder → entries; unknown folders fail like a permission error. */
function tree(t: Record<string, FileEntry[]>): Lister & { listed: string[] } {
  const listed: string[] = [];
  return {
    listed,
    async list(p) {
      listed.push(p);
      if (!(p in t)) throw new Error("Permission denied");
      return t[p];
    },
  };
}

const fs = () =>
  tree({
    "/srv": [entry("/srv/app", "dir"), entry("/srv/nginx.conf"), entry("/srv/loop", "dir", true), entry("/srv/secret", "dir")],
    "/srv/app": [entry("/srv/app/logs", "dir"), entry("/srv/app/Nginx-site.conf")],
    "/srv/app/logs": [entry("/srv/app/logs/app.log"), entry("/srv/app/logs/nginx.log")],
  });

describe("nameMatcher", () => {
  it("matches substrings case-insensitively, or wildcards against the whole name", () => {
    expect(nameMatcher("NGINX")("site-nginx.conf")).toBe(true);
    expect(nameMatcher("*.log")("app.log")).toBe(true);
    expect(nameMatcher("*.log")("app.log.gz")).toBe(false);
    expect(nameMatcher("app?.log")("app1.log")).toBe(true);
    expect(nameMatcher("a.c")("abc")).toBe(false);
    expect(nameMatcher("  ")("anything")).toBe(true);
  });
});

describe("searchTree", () => {
  it("walks subfolders breadth first, skips links and unreadable folders", async () => {
    const f = fs();
    const r = await searchTree(f, "/srv", "nginx", () => {}, new AbortController().signal);
    expect(r.matches.map((m) => m.path)).toEqual(["/srv/nginx.conf", "/srv/app/Nginx-site.conf", "/srv/app/logs/nginx.log"]);
    expect(f.listed).not.toContain("/srv/loop");
    expect(r).toMatchObject({ done: true, truncated: false, folders: 4 });
  });

  it("stops at the folder and result limits and says so", async () => {
    const r = await searchTree(fs(), "/srv", "", () => {}, new AbortController().signal, { maxFolders: 2, maxResults: 100 });
    expect(r.folders).toBe(2);
    expect(r.truncated).toBe(true);
    const few = await searchTree(fs(), "/srv", "", () => {}, new AbortController().signal, { maxFolders: 100, maxResults: 3 });
    expect(few.matches).toHaveLength(3);
    expect(few.truncated).toBe(true);
  });

  it("stops when aborted", async () => {
    const ctl = new AbortController();
    const seen: number[] = [];
    await searchTree(fs(), "/srv", "", (p) => (seen.push(p.folders), ctl.abort()), ctl.signal);
    expect(seen).toEqual([1]);
  });
});
