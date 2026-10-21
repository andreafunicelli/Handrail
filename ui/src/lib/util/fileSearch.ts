import type { FileEntry } from "../api/types";

/** What `searchTree` needs: listing one folder (SFTP or S3, as the file browser has it). */
export interface Lister {
  list(path: string): Promise<FileEntry[]>;
}

export interface SearchLimits {
  /** Folders listed before giving up: each is one SFTP round trip or one S3 request. */
  maxFolders: number;
  /** Matches kept; the walk stops once it has this many. */
  maxResults: number;
}

export const SEARCH_LIMITS: SearchLimits = { maxFolders: 400, maxResults: 300 };

export interface SearchProgress {
  /** Folders listed so far. */
  folders: number;
  matches: FileEntry[];
  /** The walk ended because of a limit, not because it ran out of folders. */
  truncated: boolean;
  done: boolean;
}

/**
 * Whether `name` matches `query`: case-insensitive; `*` matches any run of characters and
 * `?` one character; without wildcards the query matches anywhere in the name.
 */
export function nameMatcher(query: string): (name: string) => boolean {
  const q = query.trim().toLowerCase();
  if (!q) return () => true;
  if (!/[*?]/.test(q)) return (name) => name.toLowerCase().includes(q);
  const re = new RegExp("^" + q.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*").replace(/\?/g, ".") + "$");
  return (name) => re.test(name.toLowerCase());
}

/**
 * Look for names matching `query` in `root` and below, breadth first so nearby matches come
 * first. Links to folders are not followed (they can loop); folders that cannot be listed
 * (permissions) are skipped. `onProgress` gets the running state; `signal` stops the walk.
 */
export async function searchTree(
  fs: Lister,
  root: string,
  query: string,
  onProgress: (p: SearchProgress) => void,
  signal: AbortSignal,
  limits: SearchLimits = SEARCH_LIMITS,
): Promise<SearchProgress> {
  const match = nameMatcher(query);
  const queue = [root];
  const state: SearchProgress = { folders: 0, matches: [], truncated: false, done: false };
  while (queue.length && !signal.aborted) {
    if (state.folders >= limits.maxFolders || state.matches.length >= limits.maxResults) {
      state.truncated = true;
      break;
    }
    const dir = queue.shift()!;
    let entries: FileEntry[];
    try {
      entries = await fs.list(dir);
    } catch {
      entries = [];
    }
    if (signal.aborted) break;
    state.folders++;
    for (const e of entries) {
      if (match(e.name)) {
        // One more match than fits: say the list is cut, even if no folder is left to walk.
        if (state.matches.length < limits.maxResults) state.matches.push(e);
        else state.truncated = true;
      }
      if (e.kind === "dir" && !e.link) queue.push(e.path);
    }
    onProgress({ ...state, matches: [...state.matches] });
  }
  state.done = true;
  if (!signal.aborted) onProgress({ ...state, matches: [...state.matches] });
  return state;
}
