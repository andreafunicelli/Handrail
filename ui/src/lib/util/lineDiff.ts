/** A line-by-line comparison of two texts, for the "compare before overwrite" dialog. */

export interface DiffLine {
  op: "same" | "add" | "del";
  text: string;
  /** Line number in the old text (`same`, `del`), 1-based. */
  a?: number;
  /** Line number in the new text (`same`, `add`), 1-based. */
  b?: number;
}

/** A run of changes with the unchanged lines around it; `skipped` unchanged lines come before it. */
export interface DiffHunk {
  skipped: number;
  lines: DiffLine[];
}

export type DiffResult =
  | { ok: true; hunks: DiffHunk[]; added: number; removed: number; identical: boolean }
  /** `lines`: a text has more lines than `maxLines`; `work`: too different to compare cheaply. */
  | { ok: false; reason: "lines" | "work" };

export interface DiffOptions {
  /** Most lines either text may have (default 5000). */
  maxLines?: number;
  /** Most cells of the comparison table, after the common start and end are set aside (default 4M). */
  maxWork?: number;
  /** Unchanged lines shown around a change (default 3). */
  context?: number;
}

export function splitLines(text: string): string[] {
  if (!text) return [];
  const lines = text.replace(/\r\n?/g, "\n").split("\n");
  // A final newline ends the last line rather than starting an empty one.
  if (lines[lines.length - 1] === "") lines.pop();
  return lines;
}

/** Every line of `a` and `b` in order, marked kept, removed (from `a`) or added (from `b`). */
export function diffLines(a: string[], b: string[], maxWork = 4_000_000): DiffLine[] | null {
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) start++;
  let endA = a.length;
  let endB = b.length;
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) {
    endA--;
    endB--;
  }
  const n = endA - start;
  const m = endB - start;
  if (n * m > maxWork) return null;

  // Longest common subsequence of the middle parts: lcs[i][j] for a[i..], b[j..].
  const w = m + 1;
  const lcs = new Uint32Array((n + 1) * w);
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      lcs[i * w + j] = a[start + i] === b[start + j] ? lcs[(i + 1) * w + j + 1] + 1 : Math.max(lcs[(i + 1) * w + j], lcs[i * w + j + 1]);
    }
  }

  const out: DiffLine[] = [];
  for (let k = 0; k < start; k++) out.push({ op: "same", text: a[k], a: k + 1, b: k + 1 });
  let i = 0;
  let j = 0;
  while (i < n || j < m) {
    if (i < n && j < m && a[start + i] === b[start + j]) {
      out.push({ op: "same", text: a[start + i], a: start + i + 1, b: start + j + 1 });
      i++;
      j++;
    } else if (i < n && (j >= m || lcs[(i + 1) * w + j] >= lcs[i * w + j + 1])) {
      // Removed lines come before the lines that replace them.
      out.push({ op: "del", text: a[start + i], a: start + i + 1 });
      i++;
    } else {
      out.push({ op: "add", text: b[start + j], b: start + j + 1 });
      j++;
    }
  }
  for (let k = 0; k < a.length - endA; k++) out.push({ op: "same", text: a[endA + k], a: endA + k + 1, b: endB + k + 1 });
  return out;
}

/** The changes from `oldText` to `newText`, grouped into hunks with `context` lines around them. */
export function compareTexts(oldText: string, newText: string, opts: DiffOptions = {}): DiffResult {
  const { maxLines = 5000, maxWork = 4_000_000, context = 3 } = opts;
  const a = splitLines(oldText);
  const b = splitLines(newText);
  if (a.length > maxLines || b.length > maxLines) return { ok: false, reason: "lines" };
  const lines = diffLines(a, b, maxWork);
  if (!lines) return { ok: false, reason: "work" };

  const added = lines.filter((l) => l.op === "add").length;
  const removed = lines.filter((l) => l.op === "del").length;
  const hunks: DiffHunk[] = [];
  // Lines to show: every change and `context` lines either side of it.
  const keep = new Uint8Array(lines.length);
  lines.forEach((l, k) => {
    if (l.op === "same") return;
    for (let x = Math.max(0, k - context); x <= Math.min(lines.length - 1, k + context); x++) keep[x] = 1;
  });
  let skipped = 0;
  let cur: DiffHunk | null = null;
  for (let k = 0; k < lines.length; k++) {
    if (!keep[k]) {
      skipped++;
      cur = null;
      continue;
    }
    if (!cur) {
      cur = { skipped, lines: [] };
      hunks.push(cur);
      skipped = 0;
    }
    cur.lines.push(lines[k]);
  }
  // Unchanged lines after the last hunk.
  if (skipped && hunks.length) hunks.push({ skipped, lines: [] });
  return { ok: true, hunks, added, removed, identical: added === 0 && removed === 0 && oldText === newText };
}
