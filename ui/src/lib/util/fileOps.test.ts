import { describe, expect, it } from "vitest";
import type { FileEntry } from "../api/types";
import { copyBlock, copyKindOk, moveBlock, PREVIEW_MAX, previewBlock } from "./fileOps";
import { canCopy, editable, localBackend, looksBinary, s3Backend, sftpBackend } from "./fileBackends";

const file = (path: string, size = 10): FileEntry => ({ name: path.split("/").pop()!, path, kind: "file", link: false, size, modified: 1, permissions: 0o100644 });
const dir = (path: string): FileEntry => ({ ...file(path), kind: "dir", size: 0 });

const a = sftpBackend("a");
const b = sftpBackend("b");
const s = s3Backend("s");
const s2 = s3Backend("s2");
const pc = localBackend();

describe("copy capability matrix", () => {
  it.each([
    ["sftp", "sftp", "file", true],
    ["sftp", "sftp", "dir", true],
    ["sftp", "s3", "file", true],
    ["sftp", "s3", "dir", false],
    ["s3", "sftp", "file", true],
    ["s3", "sftp", "dir", false],
    ["s3", "s3", "file", false],
    ["s3", "s3", "dir", false],
    ["local", "sftp", "file", true],
    ["local", "sftp", "dir", false],
    ["sftp", "local", "file", true],
    ["sftp", "local", "dir", false],
    ["local", "s3", "file", true],
    ["s3", "local", "file", true],
    ["local", "local", "file", true],
    ["local", "local", "dir", false],
  ] as const)("%s → %s, %s: %s", (from, to, kind, ok) => {
    expect(copyKindOk(from, to, kind)).toBe(ok);
  });

  it("canCopy follows the matrix", () => {
    expect(canCopy(a, dir("/x"), b)).toBe(true);
    expect(canCopy(a, dir("/x"), s)).toBe(false);
    expect(canCopy(s, file("/bk/x"), s2)).toBe(false);
  });
});

describe("copyBlock", () => {
  it("explains why a copy is not offered", () => {
    expect(copyBlock(a, [], b, "/tmp")).toBe("none");
    expect(copyBlock(s, [file("/bk/x")], s2, "/bk2")).toBe("s3ToS3");
    expect(copyBlock(s, [file("/bk/x")], s, "/bk/y")).toBe("s3ToS3");
    expect(copyBlock(a, [file("/x"), dir("/d")], s, "/bk")).toBe("folderS3");
    expect(copyBlock(a, [file("/x")], s, "/")).toBe("notWritable");
    expect(copyBlock(a, [dir("/srv/app")], a, "/srv/app/sub")).toBe("intoItself");
  });

  it("allows the supported pairs", () => {
    expect(copyBlock(a, [file("/x"), dir("/d")], b, "/tmp")).toBeNull();
    expect(copyBlock(a, [file("/x")], s, "/bk/in")).toBeNull();
    expect(copyBlock(s, [file("/bk/x")], a, "/tmp")).toBeNull();
    // Into another folder of the same server.
    expect(copyBlock(a, [dir("/srv/app")], a, "/srv/backup")).toBeNull();
  });

  it("copies files between this computer and a server or S3, not folders", () => {
    expect(copyBlock(pc, [file("/Users/me/a.txt")], a, "/tmp")).toBeNull();
    expect(copyBlock(a, [file("/etc/hosts")], pc, "/Users/me")).toBeNull();
    expect(copyBlock(s, [file("/bk/x")], pc, "/C:/Users/me")).toBeNull();
    expect(copyBlock(pc, [dir("/Users/me/Documents")], a, "/tmp")).toBe("folderLocal");
    expect(copyBlock(a, [dir("/srv/app")], pc, "/Users/me")).toBe("folderLocal");
    // The drive list on Windows (and "/" elsewhere) takes no files.
    expect(copyBlock(a, [file("/etc/hosts")], pc, "/")).toBe("notWritable");
  });

  it("moves inside this computer, and never opens a local file in the editor", () => {
    expect(moveBlock(pc, [file("/Users/me/a.txt")], pc, "/Users/me/Documents")).toBeNull();
    expect(moveBlock(pc, [file("/Users/me/a.txt")], a, "/tmp")).toBe("otherPlace");
    expect(editable(pc, file("/Users/me/a.txt"))).toBe(false);
    expect(editable(a, file("/etc/hosts"))).toBe(true);
  });
});

describe("moveBlock", () => {
  it("only within one file system", () => {
    expect(moveBlock(a, [file("/x/f")], b, "/y")).toBe("otherPlace");
    expect(moveBlock(a, [file("/x/f")], s, "/bk")).toBe("otherPlace");
    expect(moveBlock(a, [file("/x/f")], a, "/y")).toBeNull();
    expect(moveBlock(a, [dir("/x/d")], a, "/y")).toBeNull();
  });

  it("S3: files only, same bucket", () => {
    expect(moveBlock(s, [file("/bk/a/f")], s, "/bk/b")).toBeNull();
    expect(moveBlock(s, [file("/bk/a/f")], s, "/other")).toBe("otherBucket");
    expect(moveBlock(s, [dir("/bk/a/d")], s, "/bk/b")).toBe("s3Folder");
    expect(moveBlock(s, [file("/bk/f")], s, "/")).toBe("notWritable");
  });

  it("not into the same folder or into itself", () => {
    expect(moveBlock(a, [file("/x/f")], a, "/x")).toBe("sameFolder");
    expect(moveBlock(a, [dir("/x/d")], a, "/x/d/e")).toBe("intoItself");
    expect(moveBlock(a, [], a, "/x")).toBe("none");
  });
});

describe("previewBlock", () => {
  const side = (entry: FileEntry | null, readable = true) => ({ entry, readable });
  it("offers the line comparison only for small, readable text files", () => {
    expect(previewBlock(side(file("/a.txt")), side(file("/b.txt")), looksBinary)).toBeNull();
    expect(previewBlock({ entry: null, readable: false, local: true }, side(file("/b.txt")), looksBinary)).toBe("local");
    expect(previewBlock(side(dir("/a")), side(dir("/b")), looksBinary)).toBe("folder");
    expect(previewBlock(side(file("/a.txt", PREVIEW_MAX + 1)), side(file("/b.txt")), looksBinary)).toBe("tooLarge");
    expect(previewBlock(side(file("/a.txt")), side(file("/b.txt", PREVIEW_MAX)), looksBinary)).toBeNull();
    expect(previewBlock(side(file("/a.tar.gz")), side(file("/b.tar.gz")), looksBinary)).toBe("binary");
    expect(previewBlock(side(file("/a.txt")), side(null), looksBinary)).toBe("unknown");
    expect(previewBlock(side(file("/a.txt")), side(file("/b.txt"), false), looksBinary)).toBe("unreadable");
  });
});
