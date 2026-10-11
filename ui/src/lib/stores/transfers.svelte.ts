import { api } from "../api";
import { errorMessage } from "../api/errors";
import { alreadyExists, copyBetween, type FileBackend } from "../util/fileBackends";
import { uuid } from "../util/id";

export interface Transfer {
  id: string;
  /** `FileBackend.key` of the browser that started it. */
  backend: string;
  /** A copy's destination browser (`FileBackend.key`): it lists the transfer too. */
  to?: string;
  direction: "up" | "down" | "copy";
  name: string;
  done: number;
  total: number;
  state: "running" | "done" | "failed";
  /** Local path (downloads) or remote path (uploads, copies) when done; the error when failed. */
  result: string | null;
}

/** File transfers of every file browser, with progress from "transfer://progress". */
class Transfers {
  list = $state<Transfer[]>([]);
  /** The folder each open file browser shows (by `FileBackend.key`), and its file system. */
  locations = $state.raw<Record<string, { path: string; fs: FileBackend }>>({});
  /** Bumped when a copy lands in a browser's file system, so an open browser refreshes. */
  touched = $state<Record<string, number>>({});
  private listening = false;

  private async listen() {
    if (this.listening) return;
    this.listening = true;
    await api.on("transfer://progress", (p) => {
      const t = this.list.find((x) => x.id === p.transfer_id);
      if (t && t.state === "running") Object.assign(t, { done: p.done, total: p.total });
    });
  }

  private async run(backend: string, direction: Transfer["direction"], name: string, go: (id: string) => Promise<string>, to?: string) {
    await this.listen();
    const t: Transfer = { id: uuid(), backend, to, direction, name, done: 0, total: 0, state: "running", result: null };
    this.list.push(t);
    const live = this.list[this.list.length - 1];
    try {
      live.result = await go(t.id);
      live.state = "done";
      live.done = live.total;
    } catch (e) {
      live.state = "failed";
      live.result = errorMessage(e);
      throw e;
    }
    return live.result;
  }

  download(fs: FileBackend, path: string) {
    const name = path.split("/").pop() ?? path;
    return this.run(fs.key, "down", name, (id) => fs.download(path, id));
  }

  upload(fs: FileBackend, localPath: string, dir: string) {
    const name = localPath.split(/[\\/]/).pop() ?? localPath;
    return this.run(fs.key, "up", name, (id) => fs.upload(localPath, dir, id));
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
    const out = await this.run(src.key, "copy", name, (tid) => copyBetween(src, path, dest, dir, overwrite, (id = tid)), dest.key).catch((e) => {
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
}

export const transfers = new Transfers();
