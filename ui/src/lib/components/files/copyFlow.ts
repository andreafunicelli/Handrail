import { errorMessage } from "../../api/errors";
import type { FileEntry } from "../../api/types";
import { t } from "../../i18n";
import { toasts } from "../../stores/toasts.svelte";
import { placeLabel, transfers } from "../../stores/transfers.svelte";
import { alreadyExists, type FileBackend } from "../../util/fileBackends";
import { joinPath } from "../../util/files";
import { compare } from "./compare.svelte";

/** What a destination folder holds, by name; null when it cannot be listed. */
async function listing(fs: FileBackend, dir: string, known?: FileEntry[]): Promise<Map<string, FileEntry> | null> {
  const list = known ?? (await fs.list(dir).catch(() => null));
  return list ? new Map(list.map((e) => [e.name, e])) : null;
}

/**
 * Copy `items` of `src` into `dir` of `dest`, one after the other. A name already taken there
 * (the backend says `exists`) opens the comparison; replacing copies again with overwrite.
 * Errors are toasted per item. Resolves with how many were copied and the last new path.
 */
export async function copyItems(src: FileBackend, items: FileEntry[], dest: FileBackend, dir: string) {
  let copied = 0;
  let last = "";
  for (const item of items) {
    const run = (overwrite: boolean) => transfers.copy(src, item.path, dest, dir, overwrite);
    try {
      try {
        last = await run(false);
      } catch (e) {
        if (!alreadyExists(e)) throw e;
        const there = (await listing(dest, dir))?.get(item.name) ?? null;
        const ok = await compare.ask({
          name: item.name,
          mode: "copy",
          src: { label: placeLabel(src), path: item.path, entry: item, fs: src },
          dst: { label: placeLabel(dest), path: there?.path ?? joinPath(dir, item.name), entry: there, fs: dest },
        });
        if (!ok) continue;
        last = await run(true);
      }
      copied++;
    } catch (e) {
      toasts.error(items.length > 1 ? `${item.name}: ${errorMessage(e)}` : e);
    }
  }
  return { copied, last };
}

/**
 * Upload local files into `dir` of `fs`. The backend replaces a file of the same name without
 * asking, so the folder is listed first (`known`: its entries when already on screen) and a
 * taken name opens the comparison. Resolves with how many were uploaded.
 */
export async function uploadFiles(fs: FileBackend, localPaths: string[], dir: string, known?: FileEntry[]) {
  if (!localPaths.length) return 0;
  const there = await listing(fs, dir, known);
  let done = 0;
  for (const local of localPaths) {
    const name = local.split(/[\\/]/).pop() ?? local;
    const taken = there?.get(name);
    if (taken) {
      const ok = await compare.ask({
        name,
        mode: "upload",
        src: { label: t("files.compare.thisComputer"), path: local, entry: null, fs: null },
        dst: { label: placeLabel(fs), path: taken.path, entry: taken, fs },
      });
      if (!ok) continue;
    }
    try {
      await transfers.upload(fs, local, dir);
      done++;
    } catch (e) {
      toasts.error(localPaths.length > 1 ? `${name}: ${errorMessage(e)}` : e);
    }
  }
  return done;
}
