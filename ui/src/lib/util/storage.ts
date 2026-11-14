/** localStorage helpers that never throw (private windows, blocked storage). */
export function load<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem("aht:" + key);
    return raw == null ? fallback : (JSON.parse(raw) as T);
  } catch {
    return fallback;
  }
}
export function save(key: string, value: unknown): void {
  try {
    localStorage.setItem("aht:" + key, JSON.stringify(value));
  } catch {
    /* ignore */
  }
}

/** Keys (after "aht:") that hold the user's own work rather than display preferences. */
const USER_CONTENT = ["workspaces", "recent", "pinned", "conn-order", "collapsed", "brain", "brain-models"];

/** Forget what this device keeps of the user's work outside the vault (query tabs and history
 *  of databases, workspaces, recent and pinned connections), as when the vault is deleted.
 *  Display preferences (theme, language, layout) stay. */
export function clearUserContent(store?: Storage): void {
  try {
    const s = store ?? localStorage;
    const keys: string[] = [];
    for (let i = 0; i < s.length; i++) {
      const k = s.key(i);
      if (k && (k.startsWith("handrail.db.") || (k.startsWith("aht:") && USER_CONTENT.includes(k.slice(4))))) keys.push(k);
    }
    for (const k of keys) s.removeItem(k);
  } catch {
    /* storage unavailable: nothing kept */
  }
}
