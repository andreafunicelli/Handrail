import type { Connection, Group } from "../api/types";

export interface FolderNode {
  group: Group;
  folders: FolderNode[];
  connections: Connection[];
  /** Total connections in this subtree. */
  count: number;
}

/** Build the nested folder tree. `order` maps connection id → manual sort position. */
export function buildTree(
  groups: Group[],
  connections: Connection[],
  order: Record<string, number> = {},
): { folders: FolderNode[]; root: Connection[] } {
  const byParent = new Map<string | null, Group[]>();
  const ids = new Set(groups.map((g) => g.id));
  for (const g of groups) {
    const p = g.parent_id && ids.has(g.parent_id) ? g.parent_id : null;
    (byParent.get(p) ?? byParent.set(p, []).get(p)!).push(g);
  }
  const conns = new Map<string | null, Connection[]>();
  for (const c of connections) {
    const k = c.group_id && ids.has(c.group_id) ? c.group_id : null;
    (conns.get(k) ?? conns.set(k, []).get(k)!).push(c);
  }
  const sortC = (list: Connection[]) =>
    [...list].sort((a, b) => (order[a.id] ?? 1e9) - (order[b.id] ?? 1e9) || a.name.localeCompare(b.name));
  const seen = new Set<string>();
  const build = (parent: string | null): FolderNode[] =>
    (byParent.get(parent) ?? [])
      .filter((g) => !seen.has(g.id) && seen.add(g.id))
      .sort((a, b) => a.order - b.order || a.name.localeCompare(b.name))
      .map((group) => {
        const folders = build(group.id);
        const connections = sortC(conns.get(group.id) ?? []);
        return { group, folders, connections, count: connections.length + folders.reduce((n, f) => n + f.count, 0) };
      });
  return { folders: build(null), root: sortC(conns.get(null) ?? []) };
}

/** Human folder path for a connection, e.g. "Production / eu-west". */
export function groupPath(groups: Group[], groupId: string | null): string {
  const parts: string[] = [];
  const byId = new Map(groups.map((g) => [g.id, g]));
  let cur = groupId ? byId.get(groupId) : undefined;
  let guard = 0;
  while (cur && guard++ < 32) {
    parts.unshift(cur.name);
    cur = cur.parent_id ? byId.get(cur.parent_id) : undefined;
  }
  return parts.join(" / ");
}

/** True if `candidate` is `ancestor` or nested inside it (prevents moving a folder into itself). */
export function isDescendant(groups: Group[], candidate: string, ancestor: string): boolean {
  const byId = new Map(groups.map((g) => [g.id, g]));
  let cur: string | null = candidate;
  let guard = 0;
  while (cur && guard++ < 64) {
    if (cur === ancestor) return true;
    cur = byId.get(cur)?.parent_id ?? null;
  }
  return false;
}
