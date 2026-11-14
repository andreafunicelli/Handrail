/** Query tabs and query history of a database connection, kept in the browser's local storage
 *  (per connection; SQL text only, never results). */
import { t } from "../i18n";
import { redact } from "./opslog";

export interface QueryTabDoc {
  key: string;
  name: string;
  sql: string;
}

export interface HistoryEntry {
  sql: string;
  at: number;
  ms: number;
  ok: boolean;
}

export const HISTORY_MAX = 200;

const tabsKey = (conn: string) => `handrail.db.tabs.${conn}`;
const oldSqlKey = (conn: string) => `handrail.db.sql.${conn}`;
const historyKey = (conn: string) => `handrail.db.history.${conn}`;

function read<T>(store: Storage, key: string): T | null {
  try {
    const raw = store.getItem(key);
    return raw ? (JSON.parse(raw) as T) : null;
  } catch {
    return null;
  }
}

function write(store: Storage, key: string, value: unknown) {
  try {
    store.setItem(key, JSON.stringify(value));
  } catch {
    // Storage full or blocked: tabs and history are a convenience.
  }
}

/** The tabs saved for a connection; the single editor text of earlier versions becomes the first. */
export function loadTabs(store: Storage, conn: string, newKey: () => string): { tabs: QueryTabDoc[]; current: number } {
  const saved = read<{ tabs: QueryTabDoc[]; current: number }>(store, tabsKey(conn));
  if (saved?.tabs?.length && saved.tabs.every((t) => typeof t.sql === "string" && typeof t.name === "string")) {
    return { tabs: saved.tabs, current: Math.min(Math.max(0, saved.current | 0), saved.tabs.length - 1) };
  }
  let sql = "";
  try {
    sql = store.getItem(oldSqlKey(conn)) ?? "";
  } catch {
    // Not readable: start empty.
  }
  return { tabs: [{ key: newKey(), name: t("db.queryName", { n: 1 }), sql }], current: 0 };
}

export function saveTabs(store: Storage, conn: string, tabs: QueryTabDoc[], current: number) {
  write(store, tabsKey(conn), { tabs, current });
}

/** The next free "Query N" name. */
export function nextTabName(tabs: QueryTabDoc[]): string {
  let n = tabs.length + 1;
  while (tabs.some((x) => x.name === t("db.queryName", { n }))) n++;
  return t("db.queryName", { n });
}

export function loadHistory(store: Storage, conn: string): HistoryEntry[] {
  const list = read<HistoryEntry[]>(store, historyKey(conn));
  return Array.isArray(list) ? list.filter((e) => typeof e?.sql === "string") : [];
}

/** Statements that may carry a password (CREATE/ALTER USER or ROLE, SET PASSWORD, IDENTIFIED BY). */
const CREDENTIAL_SQL = /\bidentified\b|\bpassword\b/i;

/** What history keeps of `sql`: the text, with passwords masked when it looks like it holds one
 *  (history is plain local storage, outside the vault). */
export function historySql(sql: string): string {
  return CREDENTIAL_SQL.test(sql) ? redact(sql) : sql;
}

/** Newest first; running the same SQL again moves it to the top. */
export function addHistory(store: Storage, conn: string, entry: HistoryEntry): HistoryEntry[] {
  entry = { ...entry, sql: historySql(entry.sql) };
  const list = [entry, ...loadHistory(store, conn).filter((e) => e.sql !== entry.sql)].slice(0, HISTORY_MAX);
  write(store, historyKey(conn), list);
  return list;
}

export function clearHistory(store: Storage, conn: string) {
  try {
    store.removeItem(historyKey(conn));
  } catch {
    // Nothing to clear.
  }
}
