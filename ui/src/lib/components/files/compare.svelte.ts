import type { FileEntry } from "../../api/types";
import { t } from "../../i18n";
import { ui } from "../../stores/ui.svelte";
import type { FileBackend } from "../../util/fileBackends";

/** One end of an overwrite: where the copy comes from, or the file it would replace. */
export interface CompareSide {
  /** The file system, for people ("web-01 · deploy@10.0.0.5", "This computer"). */
  label: string;
  path: string;
  /** What a listing said about it; null when not known (a local file before upload). */
  entry: FileEntry | null;
  /** Reads its text for the line comparison; null for a local file. */
  fs: FileBackend | null;
}

export interface CompareRequest {
  id: number;
  name: string;
  mode: "copy" | "upload";
  src: CompareSide;
  dst: CompareSide;
  /** The host that shows it. */
  host: number;
  resolve: (replace: boolean) => void;
}

interface Host {
  id: number;
  /** The rail comes before the file browsers: it is never inside a hidden tab. */
  rank: number;
  visible: () => boolean;
}

/**
 * "Compare before overwrite": a copy or upload whose destination name is taken asks here. One
 * visible host (the rail and each file browser mount one) shows the dialog; with none visible
 * the plain confirmation is used, so a copy never waits for a dialog nobody can see.
 */
class Compare {
  req = $state<CompareRequest | null>(null);
  private hosts: Host[] = [];
  private next = 1;

  ask(r: Omit<CompareRequest, "id" | "resolve" | "host">): Promise<boolean> {
    this.req?.resolve(false);
    const host = [...this.hosts].sort((a, b) => a.rank - b.rank).find((h) => h.visible());
    if (!host) {
      this.req = null;
      return ui.ask(t("files.copyExists", { name: r.name }), {
        body: t(r.src.entry?.kind === "dir" ? "files.copyMerge" : "files.copyReplace"),
        confirmLabel: t("files.copyOverwrite"),
        danger: true,
      });
    }
    return new Promise((resolve) => {
      this.req = { ...r, id: this.next++, host: host.id, resolve };
    });
  }

  answer(replace: boolean) {
    const r = this.req;
    this.req = null;
    r?.resolve(replace);
  }

  /** A host mounted; `leave` unmounts it. */
  register(rank: number, visible: () => boolean): { id: number; leave: () => void } {
    const id = this.next++;
    this.hosts.push({ id, rank, visible });
    return {
      id,
      leave: () => {
        this.hosts = this.hosts.filter((h) => h.id !== id);
        // Its dialog is gone with it: skip rather than wait forever.
        if (this.req?.host === id) this.answer(false);
      },
    };
  }
}

export const compare = new Compare();
