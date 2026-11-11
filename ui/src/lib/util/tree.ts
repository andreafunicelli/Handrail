import type { Connection, Group } from "../api/types";

export interface FolderNode {
  group: Group;
  folders: FolderNode[];
  connections: Connection[];
  /** Total connections in this subtree. */
  count: number;
}

const byName = (a: Group, b: Group) => a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" });

/** The order folders are shown in: their `order`, then by name ("Server 2" before
 *  "Server 10"). Everything that places folders uses it, so positions mean what is shown. */
export const compareGroups = (a: Group, b: Group) => a.order - b.order || byName(a, b);

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
      .sort(compareGroups)
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

/** The `order` every folder takes when all folders, at every level, are sorted by name
 *  ("Server 2" before "Server 10"). Only folders whose position changes are returned. */
export function alphabeticalGroupOrder(groups: Group[]): { id: string; order: number }[] {
  const ids = new Set(groups.map((g) => g.id));
  const byParent = new Map<string | null, Group[]>();
  for (const g of groups) {
    const p = g.parent_id && ids.has(g.parent_id) ? g.parent_id : null;
    (byParent.get(p) ?? byParent.set(p, []).get(p)!).push(g);
  }
  const changes: { id: string; order: number }[] = [];
  for (const siblings of byParent.values()) {
    [...siblings].sort(byName).forEach((g, i) => {
      if (g.order !== i) changes.push({ id: g.id, order: i });
    });
  }
  return changes;
}

/** Where to put folder `id` to move it one place up (-1) or down (+1) among its siblings, as
 *  the `beforeId` of `moveGroup` (null: at the end). `undefined` when it cannot move. */
export function stepTarget(groups: Group[], id: string, delta: -1 | 1): string | null | undefined {
  const g = groups.find((x) => x.id === id);
  if (!g) return undefined;
  const siblings = groups.filter((x) => x.parent_id === g.parent_id).sort(compareGroups);
  const i = siblings.findIndex((x) => x.id === id);
  if (delta < 0) return i > 0 ? siblings[i - 1].id : undefined;
  if (i < 0 || i >= siblings.length - 1) return undefined;
  return siblings[i + 2]?.id ?? null;
}
