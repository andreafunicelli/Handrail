/**
 * What can go between two file systems, for the dual-pane file manager and the copy dialog.
 * Copies use `api.copyBetween` (see `src-tauri/src/copy.rs`): server to server (files and
 * folders); S3 to a server, a server to S3, and this computer to or from a server or S3 (files
 * only); never S3 to S3. A move is only a rename inside one file system: there is no backend
 * move between hosts.
 */
import type { FileEntry } from "../api/types";
import { parentDir } from "./files";

type Kind = "sftp" | "s3" | "local";
interface Side {
  key: string;
  kind: Kind;
  writable(dir: string): boolean;
  canRename(entry: FileEntry): boolean;
}

/** Why a copy is not offered. */
export type CopyBlock = "none" | "s3ToS3" | "folderS3" | "folderLocal" | "notWritable" | "intoItself";
/** Why a move is not offered. */
export type MoveBlock = "none" | "otherPlace" | "otherBucket" | "s3Folder" | "notWritable" | "sameFolder" | "intoItself";

/** Whether an entry of this kind can be copied from `from` to `to` (the capability matrix). */
export function copyKindOk(from: Kind, to: Kind, entryKind: FileEntry["kind"]): boolean {
  if (from === "s3" && to === "s3") return false;
  if (from === "sftp" && to === "sftp") return true;
  return entryKind === "file";
}

/** Why `entries` of `from` cannot be copied into `dir` of `to`, or null when they can. */
export function copyBlock(from: Side, entries: FileEntry[], to: Side, dir: string): CopyBlock | null {
  if (!entries.length) return "none";
  if (from.kind === "s3" && to.kind === "s3") return "s3ToS3";
  if (!entries.every((e) => copyKindOk(from.kind, to.kind, e.kind))) return from.kind === "local" || to.kind === "local" ? "folderLocal" : "folderS3";
  if (!to.writable(dir)) return "notWritable";
  if (from.key === to.key && intoItself(entries, dir)) return "intoItself";
  return null;
}

/** First path segment: the bucket of an S3 path. */
const bucketOf = (p: string) => p.split("/").filter(Boolean)[0] ?? "";

/**
 * Why `entries` of `from` cannot be moved into `dir` of `to`, or null when they can: only
 * inside the same file system (a rename), and on S3 only files within one bucket.
 */
export function moveBlock(from: Side, entries: FileEntry[], to: Side, dir: string): MoveBlock | null {
  if (!entries.length) return "none";
  if (from.key !== to.key) return "otherPlace";
  if (!to.writable(dir) || !entries.every((e) => from.writable(parentDir(e.path)))) return "notWritable";
  if (entries.every((e) => parentDir(e.path) === dir)) return "sameFolder";
  if (from.kind === "s3") {
    if (!entries.every((e) => from.canRename(e))) return "s3Folder";
    if (!entries.every((e) => bucketOf(e.path) === bucketOf(dir))) return "otherBucket";
  }
  if (intoItself(entries, dir)) return "intoItself";
  return null;
}

/** A folder among `entries` that `dir` is, or is inside of. */
const intoItself = (entries: FileEntry[], dir: string) => entries.some((e) => e.kind === "dir" && (dir === e.path || dir.startsWith(e.path + "/")));

/** Largest file either side of an overwrite may be for the line comparison. */
export const PREVIEW_MAX = 256 * 1024;

/** Why the two ends of an overwrite cannot be compared line by line. */
export type PreviewBlock = "local" | "folder" | "unknown" | "tooLarge" | "binary" | "unreadable";

interface PreviewSide {
  entry: FileEntry | null;
  /** Can read text (`FileBackend.readText`); false for a local file. */
  readable: boolean;
  local?: boolean;
}

/**
 * Whether the line comparison can be offered without fetching anything big: both ends are
 * listed files of at most `PREVIEW_MAX` bytes, readable as text, with no binary-looking name.
 */
export function previewBlock(src: PreviewSide, dst: PreviewSide, looksBinary: (name: string) => boolean): PreviewBlock | null {
  if (src.local || dst.local) return "local";
  if (!src.entry || !dst.entry) return "unknown";
  if (src.entry.kind === "dir" || dst.entry.kind === "dir") return "folder";
  if (src.entry.kind !== "file" || dst.entry.kind !== "file") return "unknown";
  if (src.entry.size > PREVIEW_MAX || dst.entry.size > PREVIEW_MAX) return "tooLarge";
  if (looksBinary(src.entry.name) || looksBinary(dst.entry.name)) return "binary";
  if (!src.readable || !dst.readable) return "unreadable";
  return null;
}
