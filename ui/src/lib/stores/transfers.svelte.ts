import { api } from "../api";
import { errorMessage } from "../api/errors";
import { uuid } from "../util/id";

export interface Transfer {
  id: string;
  sessionId: string;
  direction: "up" | "down";
  name: string;
  done: number;
  total: number;
  state: "running" | "done" | "failed";
  /** Local path (downloads) or remote path (uploads) when done; the error when failed. */
  result: string | null;
}

/** File transfers of every file browser, with progress from "sftp://progress". */
class Transfers {
  list = $state<Transfer[]>([]);
  private listening = false;

  private async listen() {
    if (this.listening) return;
    this.listening = true;
    await api.on("sftp://progress", (p) => {
      const t = this.list.find((x) => x.id === p.transfer_id);
      if (t && t.state === "running") Object.assign(t, { done: p.done, total: p.total });
    });
  }

  private async run(sessionId: string, direction: Transfer["direction"], name: string, go: (id: string) => Promise<string>) {
    await this.listen();
    const t: Transfer = { id: uuid(), sessionId, direction, name, done: 0, total: 0, state: "running", result: null };
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

  download(sessionId: string, path: string) {
    const name = path.split("/").pop() ?? path;
    return this.run(sessionId, "down", name, (id) => api.sftpDownload(sessionId, path, id));
  }

  upload(sessionId: string, localPath: string, dir: string) {
    const name = localPath.split(/[\\/]/).pop() ?? localPath;
    return this.run(sessionId, "up", name, (id) => api.sftpUpload(sessionId, localPath, dir, id));
  }

  /** Forget finished transfers of a session. */
  clear(sessionId: string) {
    this.list = this.list.filter((t) => t.sessionId !== sessionId || t.state === "running");
  }
}

export const transfers = new Transfers();
