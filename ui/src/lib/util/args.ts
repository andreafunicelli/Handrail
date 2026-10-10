/**
 * Split a command-line argument string the way a user expects from a shell, without shell
 * expansion. Whitespace separates arguments; `'…'` is literal; inside `"…"` only `\"` and `\\` are
 * escapes. Backslashes elsewhere are literal so Windows paths (`C:\tools\x`) survive.
 * Returns `null` for an unterminated quote.
 */
export function splitArgs(input: string): string[] | null {
  const out: string[] = [];
  let cur = "";
  let inArg = false;
  for (let i = 0; i < input.length; i++) {
    const c = input[i];
    if (c === "'" || c === '"') {
      const end = c === "'" ? input.indexOf("'", i + 1) : closingDoubleQuote(input, i + 1);
      if (end < 0) return null;
      const body = input.slice(i + 1, end);
      cur += c === "'" ? body : body.replace(/\\(["\\])/g, "$1");
      inArg = true;
      i = end;
    } else if (/\s/.test(c)) {
      if (inArg) out.push(cur);
      cur = "";
      inArg = false;
    } else {
      cur += c;
      inArg = true;
    }
  }
  if (inArg) out.push(cur);
  return out;
}

function closingDoubleQuote(s: string, from: number): number {
  for (let i = from; i < s.length; i++) {
    if (s[i] === "\\" && (s[i + 1] === '"' || s[i + 1] === "\\")) i++;
    else if (s[i] === '"') return i;
  }
  return -1;
}

/** Inverse of {@link splitArgs}: `splitArgs(joinArgs(a))` equals `a`. */
export function joinArgs(args: string[]): string {
  return args
    .map((a) => {
      if (a !== "" && !/[\s'"]/.test(a)) return a;
      if (!a.includes("'")) return `'${a}'`;
      return `"${a.replace(/(["\\])/g, "\\$1")}"`;
    })
    .join(" ");
}
