import { api } from "../api";
import type { FileEntry } from "../api/types";

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
  /**
   * The same file system as root (SFTP only): starts the server through sudo. Rejects with
   * `sudo_password_required` when `password` is needed.
   */
  asRoot?(password: string | null): Promise<FileBackend>;
}

export function sftpBackend(sessionId: string, root = false): FileBackend {
  return {
    // One transfer list for both identities of the browser.
    key: `sftp:${sessionId}`,
    kind: "sftp",
    root,
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
  };
}
