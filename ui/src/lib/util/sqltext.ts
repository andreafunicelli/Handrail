/** Splitting SQL text into statements the way the server reads it: semicolons inside strings,
 *  quoted names and comments do not end a statement. */

export type SqlDialect = "mysql" | "postgres";

export interface StatementSpan {
  /** Offsets in the text, the statement's own text without the closing `;`. */
  from: number;
  to: number;
  text: string;
}

/** The statements of `sql`, trimmed; empty ones (only blanks or comments) are left out. */
export function splitStatements(sql: string, dialect: SqlDialect): StatementSpan[] {
  const out: StatementSpan[] = [];
  const mysql = dialect === "mysql";
  let start = 0;
  let i = 0;
  let code = false; // the current statement holds more than blanks and comments
  const push = (end: number) => {
    if (code) {
      const raw = sql.slice(start, end);
      const lead = raw.length - raw.trimStart().length;
      const text = raw.trim();
      out.push({ from: start + lead, to: start + lead + text.length, text });
    }
    code = false;
  };
  while (i < sql.length) {
    const c = sql[i];
    const next = sql[i + 1];
    if (c === "-" && next === "-" && (!mysql || i + 2 >= sql.length || /\s/.test(sql[i + 2]))) {
      while (i < sql.length && sql[i] !== "\n") i++;
      continue;
    }
    if (mysql && c === "#") {
      while (i < sql.length && sql[i] !== "\n") i++;
      continue;
    }
    if (c === "/" && next === "*") {
      const end = sql.indexOf("*/", i + 2);
      i = end < 0 ? sql.length : end + 2;
      continue;
    }
    if (c === "'" || c === '"' || (mysql && c === "`")) {
      code = true;
      const escapes = mysql && c !== "`";
      i++;
      while (i < sql.length) {
        if (escapes && sql[i] === "\\") {
          i += 2;
          continue;
        }
        if (sql[i] === c) {
          if (sql[i + 1] === c) {
            i += 2;
            continue;
          }
          i++;
          break;
        }
        i++;
      }
      continue;
    }
    if (!mysql && c === "$") {
      const tag = /^\$[A-Za-z_]?[A-Za-z0-9_]*\$/.exec(sql.slice(i));
      if (tag && !/^\$\d/.test(sql.slice(i))) {
        code = true;
        const end = sql.indexOf(tag[0], i + tag[0].length);
        i = end < 0 ? sql.length : end + tag[0].length;
        continue;
      }
    }
    if (c === ";") {
      push(i);
      start = i + 1;
      i++;
      continue;
    }
    if (!/\s/.test(c)) code = true;
    i++;
  }
  push(sql.length);
  return out;
}

/** The statement the cursor is in; between statements, the one just before it (else after). */
export function statementAt(sql: string, pos: number, dialect: SqlDialect): StatementSpan | null {
  const all = splitStatements(sql, dialect);
  if (!all.length) return null;
  const inside = all.find((s) => pos >= s.from && pos <= s.to + 1);
  if (inside) return inside;
  const before = all.filter((s) => s.to < pos);
  return before.length ? before[before.length - 1] : all[0];
}
