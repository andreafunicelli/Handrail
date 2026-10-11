import { api } from "../api";
import { errorMessage } from "../api/errors";
import type { FileBackend } from "../util/fileBackends";
import { uuid } from "../util/id";

export interface Transfer {
  id: string;
  /** `FileBackend.key` of the browser that started it. */
  backend: string;
  direction: "up" | "down";
  name: string;
  done: number;
  total: number;
  state: "running" | "done" | "failed";
  /** Local path (downloads) or remote path (uploads) when done; the error when failed. */
  result: string | null;
}

/** File transfers of every file browser, with progress from "transfer://progress". */
class Transfers {
  list = $state<Transfer[]>([]);
  private listening = false;

  private async listen() {
    if (this.listening) return;
    this.listening = true;
    await api.on("transfer://progress", (p) => {
      const t = this.list.find((x) => x.id === p.transfer_id);
      if (t && t.state === "running") Object.assign(t, { done: p.done, total: p.total });
    });
  }

  private async run(backend: string, direction: Transfer["direction"], name: string, go: (id: string) => Promise<string>) {
    await this.listen();
    const t: Transfer = { id: uuid(), backend, direction, name, done: 0, total: 0, state: "running", result: null };
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

  /** Forget the finished transfers of one browser. */
  clear(backend: string) {
    this.list = this.list.filter((t) => t.backend !== backend || t.state === "running");
  }
}

export const transfers = new Transfers();
