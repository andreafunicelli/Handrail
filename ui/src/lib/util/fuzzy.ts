/**
 * Small subsequence fuzzy matcher. Returns a score (higher is better) or null when `query`
 * is not a subsequence of `text`. Rewards consecutive runs, word starts and early matches.
 */
export function fuzzyScore(query: string, text: string): number | null {
  const q = query.trim().toLowerCase();
  if (!q) return 0;
  const t = text.toLowerCase();
  let score = 0;
  let ti = 0;
  let prev = -2;
  for (let qi = 0; qi < q.length; qi++) {
    const ch = q[qi];
    if (ch === " ") continue;
    const idx = t.indexOf(ch, ti);
    if (idx === -1) return null;
    let s = 1;
    if (idx === prev + 1) s += 4;
    if (idx === 0 || /[\s\-_.@/:]/.test(t[idx - 1])) s += 3;
    s -= Math.min(idx - ti, 5) * 0.2;
    score += s;
    prev = idx;
    ti = idx + 1;
  }
  if (t.startsWith(q)) score += 5;
  return score - t.length * 0.01;
}

/** Filter + sort `items` by the best score across the fields returned by `fields`. */
export function fuzzyFilter<T>(items: T[], query: string, fields: (item: T) => string[]): T[] {
  if (!query.trim()) return items;
  const scored: { item: T; score: number }[] = [];
  for (const item of items) {
    let best: number | null = null;
    for (const f of fields(item)) {
      const s = fuzzyScore(query, f);
      if (s != null && (best == null || s > best)) best = s;
    }
    if (best != null) scored.push({ item, score: best });
  }
  return scored.sort((a, b) => b.score - a.score).map((s) => s.item);
}
