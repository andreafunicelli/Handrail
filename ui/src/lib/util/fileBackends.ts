import { api } from "../api";
import type { FileEntry, FileStamp, Place, TextFile } from "../api/types";

/** A real folder, not a link to one (deleting a link removes only the link). */
export const isFolder = (e: FileEntry) => e.kind === "dir" && !e.link;

/** What the file browser needs from a remote file system (SFTP on a session, or S3). */
export interface FileBackend {
  /** Identifies the browser's transfers. */
  key: string;
  kind: "sftp" | "s3";
  home(): Promise<string>;
  list(path: string): Promise<FileEntry[]>;
  mkdir(path: string): Promise<void>;
  rename(entry: FileEntry, to: string): Promise<void>;
  remove(entry: FileEntry): Promise<void>;
  /** Resolves with the local path in Downloads. */
  download(path: string, transferId: string): Promise<string>;
  /** Resolves with the remote path. */
  upload(localPath: string, dir: string, transferId: string): Promise<string>;
  /** Whether `dir` can hold files and folders (not the S3 bucket list). */
  writable(dir: string): boolean;
  /** S3 folders are key prefixes and cannot be renamed. */
  canRename(entry: FileEntry): boolean;
  /** Running as root through sudo. */
  root?: boolean;
  /** The SSH session these files are on (SFTP): the agent works there. */
  sessionId?: string;
  /** Text editing (SFTP only). */
  readText?(path: string): Promise<TextFile>;
  /** Saves unless the file changed since `expected` (null: a new file); see `api.sftpWriteText`. */
  writeText?(path: string, text: string, expected: FileStamp | null, backup: boolean): Promise<FileStamp>;
  /**
   * The same file system as root (SFTP only): starts the server through sudo. Rejects with
   * `sudo_password_required` when `password` is needed.
   */
  asRoot?(password: string | null): Promise<FileBackend>;
  /** Leaving root mode (the root backend's counterpart of `asRoot`). */
  leaveRoot?(): Promise<void>;
  /** This file system as one end of a copy (`api.copyBetween`). */
  place: Place;
}

/**
 * Whether `entry` of `from` can be copied to `to`: server to server (files and folders), and
 * between a server and S3 (files). Not between S3 accounts.
 */
export function canCopy(from: FileBackend, entry: FileEntry, to: FileBackend): boolean {
  if (from.kind === "s3" && to.kind === "s3") return false;
  return from.kind === "sftp" && to.kind === "sftp" ? true : entry.kind === "file";
}

/** Copy `path` into `dir` of `dest`; resolves with the new path, rejects with `exists` (see `alreadyExists`). */
export const copyBetween = (src: FileBackend, path: string, dest: FileBackend, dir: string, overwrite: boolean, transferId: string) =>
  api.copyBetween(src.place, path, dest.place, dir, overwrite, transferId);

export function sftpBackend(sessionId: string, root = false): FileBackend {
  return {
    // One transfer list for both identities of the browser.
    key: `sftp:${sessionId}`,
    kind: "sftp",
    root,
    sessionId,
    readText: (p) => api.sftpReadText(sessionId, p, root),
    writeText: (p, text, expected, backup) => api.sftpWriteText(sessionId, p, text, expected, backup, root),
    home: () => api.sftpHome(sessionId, root),
    list: (p) => api.sftpList(sessionId, p, root),
    mkdir: (p) => api.sftpMkdir(sessionId, p, root),
    rename: (e, to) => api.sftpRename(sessionId, e.path, to, root),
    remove: (e) => api.sftpRemove(sessionId, e.path, isFolder(e), root),
    download: (p, id) => api.sftpDownload(sessionId, p, id, root),
    upload: (local, dir, id) => api.sftpUpload(sessionId, local, dir, id, root),
    writable: (dir) => !!dir,
    canRename: () => true,
    asRoot: async (password) => {
      await api.sftpSudo(sessionId, password);
      return sftpBackend(sessionId, true);
    },
    leaveRoot: root ? () => api.sftpRootOff(sessionId) : undefined,
    place: { kind: "sftp", session_id: sessionId, root },
  };
}

/** The error `asRoot` rejects with when the sudo password has to be typed. */
export const needsSudoPassword = (e: unknown) =>
  !!e && typeof e === "object" && (e as { message?: unknown }).message === "sudo_password_required";

/** "/" is the bucket list; files live at "/bucket/…". */
export function s3Backend(accountId: string): FileBackend {
  return {
    key: `s3:${accountId}`,
    kind: "s3",
    home: () => api.s3Home(accountId),
    list: (p) => api.s3List(accountId, p),
    mkdir: (p) => api.s3Mkdir(accountId, p),
    rename: (e, to) => api.s3Rename(accountId, e.path, to, e.kind === "dir"),
    remove: (e) => api.s3Remove(accountId, e.path, e.kind === "dir"),
    download: (p, id) => api.s3Download(accountId, p, id),
    upload: (local, dir, id) => api.s3Upload(accountId, local, dir, id),
    writable: (dir) => dir.split("/").filter(Boolean).length > 0,
    canRename: (e) => e.kind !== "dir",
    place: { kind: "s3", account_id: accountId },
  };
}

/** The error a copy rejects with when its destination name is taken. */
export const alreadyExists = (e: unknown) => !!e && typeof e === "object" && (e as { message?: unknown }).message === "exists";

/** The error a save rejects with when the file changed on the server since it was read. */
export const changedOnServer = (e: unknown) =>
  !!e && typeof e === "object" && (e as { message?: unknown }).message === "changed_on_server";

/** Files that are never text: double-click downloads them instead of opening the editor. */
const BINARY = /\.(gz|tgz|bz2|xz|zst|zip|7z|rar|tar|iso|img|qcow2|deb|rpm|apk|jar|war|so|o|a|bin|exe|dll|class|pyc|png|jpe?g|gif|webp|ico|bmp|tiff?|pdf|mp[34]|mkv|mov|avi|wav|flac|ogg|woff2?|ttf|otf|sqlite|db)$/i;

/** Whether double-clicking `e` should open the editor. */
export function editable(fs: FileBackend, e: FileEntry): boolean {
  return !!fs.readText && e.kind === "file" && e.size <= 5 * 1024 * 1024 && !BINARY.test(e.name);
}
