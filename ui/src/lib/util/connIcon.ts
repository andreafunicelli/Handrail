import type { IconName } from "../components/icons/Icon.svelte";

/** Built-in icons offered for connections, in picker order. */
export const CONNECTION_ICONS: IconName[] = [
  "server", "monitor", "terminal", "database", "globe", "cloud", "box", "cpu", "bucket",
  "git", "mail", "chart", "wifi", "home", "flask", "shield", "lock", "key", "bolt", "sparkle",
];

/** A typed icon is kept only if it is a single character as people see it (one emoji, one
 *  letter); anything longer would push the name out of the sidebar. Returns null otherwise. */
export function singleGlyph(text: string): string | null {
  const s = text.trim();
  if (!s) return null;
  const Seg = (Intl as unknown as { Segmenter?: new (l?: string, o?: { granularity: string }) => { segment(s: string): Iterable<unknown> } }).Segmenter;
  const count = Seg ? [...new Seg(undefined, { granularity: "grapheme" }).segment(s)].length : [...s].length;
  return count === 1 ? s : null;
}
