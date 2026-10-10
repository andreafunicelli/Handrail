/**
 * Deny-list entries as the user types them vs. as the core stores them.
 *
 * The core (`aht-term` policy) compiles every stored entry with Rust's `regex` crate and silently
 * drops entries that fail to compile. The UI therefore stores only valid Rust regexes:
 * - plain text is escaped, so it matches as a literal substring;
 * - `/pattern/flags` is a regular expression; `i`, `m` and `s` become inline flags (`(?i)`).
 */

/** The characters `regex::escape` escapes. */
const META = /[\\.+*?()|[\]{}^$#&\-~]/g;

export function escapeRegex(text: string): string {
  return text.replace(META, "\\$&");
}

/** The literal text a pattern matches, if it is nothing but escaped literal text. */
export function literalText(pattern: string): string | null {
  let out = "";
  for (let i = 0; i < pattern.length; i++) {
    const c = pattern[i];
    if (c === "\\") {
      const next = pattern[i + 1];
      if (next === undefined || !"\\.+*?()|[]{}^$#&-~".includes(next)) return null;
      out += next;
      i++;
    } else if ("\\.+*?()|[]{}^$#&-~".includes(c)) {
      // `-`, `#`, `&` and `~` are only special inside classes or with flags; unescaped they are
      // still literal at the top level.
      if (!"-#&~".includes(c)) return null;
      out += c;
    } else {
      out += c;
    }
  }
  return out;
}

export type DenyInput = { ok: true; pattern: string } | { ok: false; error: "empty" | "unsupported" | "invalid" };

/**
 * `/body/flags` where every flag is a JS regex flag. Anything else with slashes (`/etc/shadow`,
 * `/usr/bin`) is a path and stays literal text.
 */
const SLASHED = /^\/(.+)\/([dgimsuvy]*)$/s;
/** Flags Rust's regex supports inline; the rest (g, y, d, u, v) do not change a match test. */
const INLINE_FLAGS = "ims";

export function isSlashed(input: string): boolean {
  return SLASHED.test(input.trim());
}

/** Turn what the user typed into a stored (Rust regex) pattern. */
export function parseDenyInput(input: string): DenyInput {
  const text = input.trim();
  if (!text) return { ok: false, error: "empty" };
  const m = SLASHED.exec(text);
  if (!m) return { ok: true, pattern: escapeRegex(text) };
  // `\/` is how JS regex literals write a slash; Rust's regex has no need to escape it.
  const body = m[1].replace(/\\\//g, "/");
  const flags = [...new Set(m[2])];
  // Look-around and backreferences are JS-only; the Rust engine rejects them.
  if (/\(\?<?[=!]|\\[1-9]|\\k</.test(body)) return { ok: false, error: "unsupported" };
  try {
    // JS cannot parse Rust's leading inline flags; check the rest.
    new RegExp(body.replace(/^\(\?[imsUux]+\)/, ""));
  } catch {
    return { ok: false, error: "invalid" };
  }
  const inline = flags.filter((f) => INLINE_FLAGS.includes(f)).sort().join("");
  return { ok: true, pattern: inline ? `(?${inline})${body}` : body };
}

/**
 * Entries saved while the UI advertised `/regex/` notation the core did not parse: convert them
 * to what they meant. Returns the list unchanged (same array) when nothing needed converting.
 */
export function normalizeDenyList(list: string[]): string[] {
  let changed = false;
  const out: string[] = [];
  for (const p of list) {
    let next = p;
    if (isSlashed(p)) {
      const r = parseDenyInput(p);
      if (r.ok) next = r.pattern;
    }
    if (next !== p) changed = true;
    if (!out.includes(next)) out.push(next);
    else changed = true;
  }
  return changed ? out : list;
}
