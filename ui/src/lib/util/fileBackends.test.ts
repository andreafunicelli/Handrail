import { describe, expect, it, vi } from "vitest";
import type { FileEntry } from "../api/types";

const tree: Record<string, FileEntry[]> = {};
const removed: string[] = [];
vi.mock("../api", () => ({
  api: {
    sftpList: async (_s: string, p: string) => tree[p] ?? [],
    sftpRemove: async (_s: string, p: string) => void removed.push(p),
  },
}));
const { sftpBackend } = await import("./fileBackends");

const entry = (path: string, kind: FileEntry["kind"], link = false): FileEntry => ({
  name: path.split("/").pop()!,
  path,
  kind,
  link,
  size: 0,
  modified: null,
  permissions: null,
});

describe("sftp remove", () => {
  it("empties a folder deepest first and does not follow links", async () => {
    tree["/app"] = [entry("/app/a.txt", "file"), entry("/app/sub", "dir"), entry("/app/shared", "dir", true)];
    tree["/app/sub"] = [entry("/app/sub/b.txt", "file")];
    tree["/app/shared"] = [entry("/app/shared/keep.txt", "file")];
    await sftpBackend("s1").remove(entry("/app", "dir"));
    expect(removed).toEqual(["/app/a.txt", "/app/sub/b.txt", "/app/sub", "/app/shared", "/app"]);
  });
});
