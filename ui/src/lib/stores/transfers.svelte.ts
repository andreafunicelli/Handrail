import { api } from "../api";
import { errorMessage } from "../api/errors";
import { t } from "../i18n";
import { alreadyExists, copyBetween, type FileBackend } from "../util/fileBackends";
import { uuid } from "../util/id";
import { joinPath } from "../util/files";
import { progressed, runningCount, settle, trimmed, type Transfer } from "../util/transfers";
import { s3 } from "./s3.svelte";
import { sessions } from "./sessions.svelte";

export type { Transfer } from "../util/transfers";

/** A file system's name for people: the session's title and user@host, or the S3 account. */
export function placeLabel(fs: FileBackend): string {
  if (fs.place.kind === "s3") return `${s3.byId(fs.place.account_id)?.name ?? "S3"} (S3)`;
  const i = sessions.infos[fs.place.session_id];
  const who = i ? `${i.user ?? ""}@${i.host ?? ""}` : "SFTP";
  return `${i?.title && i.title !== i.host ? `${i.title} · ` : ""}${who}${fs.root ? " (root)" : ""}`;
}

/**
 * File transfers of every file browser, with progress from "transfer://progress". In memory
 * only: the list starts empty with each launch.
 */
class Transfers {
  list = $state<Transfer[]>([]);
  /** The folder each open file browser shows (by `FileBackend.key`), and its file system. */
  locations = $state.raw<Record<string, { path: string; fs: FileBackend }>>({});
  /** Bumped when a copy lands in a browser's file system, so an open browser refreshes. */
  touched = $state<Record<string, number>>({});
  /** Transfers still running, for the badge on the rail. */
  running = $derived(runningCount(this.list));
  /** The global transfer panel is open. */
  panelOpen = $state(false);
  private listening = false;

  private async listen() {
    if (this.listening) return;
    this.listening = true;
    await api.on("transfer://progress", (p) => {
      const i = this.list.findIndex((x) => x.id === p.transfer_id);
      if (i >= 0) this.list[i] = progressed(this.list[i], p.done, p.total);
    });
  }

  private replace(id: string, f: (t: Transfer) => Transfer) {
    const i = this.list.findIndex((x) => x.id === id);
    if (i >= 0) this.list[i] = f(this.list[i]);
    return i >= 0 ? this.list[i] : null;
  }

  private async run(
    rec: Pick<Transfer, "backend" | "to" | "direction" | "name" | "source" | "dest">,
    go: (id: string) => Promise<string>,
  ) {
    await this.listen();
    const t: Transfer = { ...rec, id: uuid(), done: 0, total: 0, state: "running", result: null, started: Date.now() };
    this.list = [...trimmed(this.list), t];
    try {
      const value = await go(t.id);
      this.replace(t.id, (x) => settle(x, { ok: true, value }));
      return value;
    } catch (e) {
      this.replace(t.id, (x) => settle(x, { ok: false, error: errorMessage(e) }));
      throw e;
    }
  }

  download(fs: FileBackend, path: string) {
    const name = path.split("/").pop() ?? path;
    const source = `${placeLabel(fs)}: ${path}`;
    return this.run({ backend: fs.key, direction: "down", name, source, dest: t("files.transfers.downloads") }, (id) => fs.download(path, id));
  }

  upload(fs: FileBackend, localPath: string, dir: string) {
    const name = localPath.split(/[\\/]/).pop() ?? localPath;
    return this.run({ backend: fs.key, direction: "up", name, source: localPath, dest: `${placeLabel(fs)}: ${joinPath(dir, name)}` }, (id) =>
      fs.upload(localPath, dir, id),
    );
  }

  /** A browser of `fs` now shows `path`. */
  located(fs: FileBackend, path: string) {
    this.locations = { ...this.locations, [fs.key]: { path, fs } };
  }

  /** A browser of `fs` closed. */
  gone(fs: FileBackend) {
    if (this.locations[fs.key]?.fs !== fs) return;
    const { [fs.key]: _, ...rest } = this.locations;
    this.locations = rest;
  }

  /** Copy `path` from `src` into `dir` of `dest` (server to server, S3 to a server, a server to S3). */
  async copy(src: FileBackend, path: string, dest: FileBackend, dir: string, overwrite: boolean) {
    const name = path.split("/").pop() ?? path;
    let id = "";
    const rec = { backend: src.key, to: dest.key, direction: "copy" as const, name, source: `${placeLabel(src)}: ${path}`, dest: `${placeLabel(dest)}: ${joinPath(dir, name)}` };
    const out = await this.run(rec, (tid) => copyBetween(src, path, dest, dir, overwrite, (id = tid))).catch((e) => {
      // Not a failure: the caller asks whether to replace it.
      if (alreadyExists(e)) this.list = this.list.filter((x) => x.id !== id);
      throw e;
    });
    this.touched[dest.key] = (this.touched[dest.key] ?? 0) + 1;
    return out;
  }

  /** Transfers a browser shows: those it started, and copies into it. */
  of(backend: string) {
    return this.list.filter((t) => t.backend === backend || t.to === backend);
  }

  /** Forget the finished transfers of one browser. */
  clear(backend: string) {
    this.list = this.list.filter((t) => (t.backend !== backend && t.to !== backend) || t.state === "running");
  }

  /** Forget every finished transfer (the panel's "Clear finished"). */
  clearFinished() {
    this.list = this.list.filter((t) => t.state === "running");
  }
}

export const transfers = new Transfers();
