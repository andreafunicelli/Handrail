import type { FileEntry } from "../../api/types";
import type { FileBackend } from "../../util/fileBackends";

/** What one pane of the file manager shows, for the operations between the two panes. */
export class PaneHandle {
  /** Its file system (as root while the pane's root toggle is on). */
  fs = $state.raw<FileBackend | null>(null);
  path = $state("");
  entries = $state.raw<FileEntry[]>([]);
  /** The selected rows on screen (never the rows a search hides). */
  picked = $state.raw<FileEntry[]>([]);
  /** Lists the folder again. */
  refresh: () => Promise<void> = async () => {};
  /** Puts the keyboard focus on the pane's list (the selected row, else the first). */
  focus: () => void = () => {};
}
