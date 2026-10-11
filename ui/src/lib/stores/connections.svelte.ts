import { api } from "../api";
import type { Connection, Group } from "../api/types";
import { uuid } from "../util/id";
import { load, save } from "../util/storage";
import { buildTree, isDescendant } from "../util/tree";
import type { QuickTarget } from "../util/quickconnect";

export function blankConnection(groupId: string | null = null): Connection {
  return {
    id: uuid(),
    name: "",
    kind: "ssh",
    group_id: groupId,
    host: "",
    port: 22,
    user: "",
    auth: { type: "agent" },
    jump: [],
    env: "none",
    tags: [],
    color: null,
    approval_mode: null,
    local_shell: null,
    notes: "",
    tunnels: [],
  };
}

class Connections {
  list = $state<Connection[]>([]);
  groups = $state<Group[]>([]);
  loaded = $state(false);
  /** Manual sort order inside folders (UI-side until Connection gains an `order` field). */
  order = $state<Record<string, number>>(load("conn-order", {}));
  collapsed = $state<Record<string, boolean>>(load("collapsed", {}));
  pinned = $state<string[]>(load("pinned", []));
  recent = $state<string[]>(load("recent", []));

  tree = $derived(buildTree(this.groups, this.list, this.order));
  byId = $derived(new Map(this.list.map((c) => [c.id, c])));

  async load() {
    const [list, groups] = await Promise.all([api.listConnections(), api.listGroups()]);
    this.list = list;
    this.groups = groups;
    this.loaded = true;
  }

  async save(c: Connection) {
    const saved = await api.saveConnection($state.snapshot(c) as Connection);
    const i = this.list.findIndex((x) => x.id === saved.id);
    if (i >= 0) this.list[i] = saved;
    else this.list.push(saved);
    return saved;
  }

  async remove(id: string) {
    await api.deleteConnection(id);
    this.list = this.list.filter((c) => c.id !== id);
    this.pinned = this.pinned.filter((p) => p !== id);
    this.recent = this.recent.filter((p) => p !== id);
    this.persist();
  }

  async duplicate(id: string, suffix: string) {
    const c = this.byId.get(id);
    if (!c) return;
    const copy = { ...structuredClone($state.snapshot(c) as Connection), id: uuid(), name: `${c.name} ${suffix}` };
    return this.save(copy);
  }

  async saveGroup(g: Group) {
    const saved = await api.saveGroup($state.snapshot(g) as Group);
    const i = this.groups.findIndex((x) => x.id === saved.id);
    if (i >= 0) this.groups[i] = saved;
    else this.groups.push(saved);
    return saved;
  }

  async newGroup(name: string, parent: string | null = null) {
    const siblings = this.groups.filter((g) => g.parent_id === parent);
    return this.saveGroup({ id: uuid(), name, parent_id: parent, order: siblings.length });
  }

  async removeGroup(id: string) {
    await api.deleteGroup(id);
    this.groups = this.groups.filter((g) => g.id !== id).map((g) => (g.parent_id === id ? { ...g, parent_id: null } : g));
    this.list = this.list.map((c) => (c.group_id === id ? { ...c, group_id: null } : c));
  }

  /** Move a connection into `groupId`, optionally placing it before `beforeId`. */
  async moveConnection(id: string, groupId: string | null, beforeId: string | null = null) {
    const c = this.byId.get(id);
    if (!c) return;
    const siblings = this.list
      .filter((x) => x.group_id === groupId && x.id !== id)
      .sort((a, b) => (this.order[a.id] ?? 1e9) - (this.order[b.id] ?? 1e9) || a.name.localeCompare(b.name));
    const idx = beforeId ? siblings.findIndex((s) => s.id === beforeId) : -1;
    siblings.splice(idx < 0 ? siblings.length : idx, 0, c);
    const order = { ...this.order };
    siblings.forEach((s, i) => (order[s.id] = i));
    this.order = order;
    this.persist();
    if (c.group_id !== groupId) await this.save({ ...$state.snapshot(c), group_id: groupId } as Connection);
  }

  /** Move a folder under `parentId` (null = top level), refusing cycles. */
  async moveGroup(id: string, parentId: string | null, beforeId: string | null = null) {
    if (parentId && isDescendant(this.groups, parentId, id)) return;
    const g = this.groups.find((x) => x.id === id);
    if (!g) return;
    const siblings = this.groups.filter((x) => x.parent_id === parentId && x.id !== id).sort((a, b) => a.order - b.order);
    const idx = beforeId ? siblings.findIndex((s) => s.id === beforeId) : -1;
    siblings.splice(idx < 0 ? siblings.length : idx, 0, g);
    for (const [i, s] of siblings.entries()) {
      if (s.order !== i || s.id === id) await this.saveGroup({ ...$state.snapshot(s), order: i, parent_id: parentId } as Group);
    }
  }

  toggleCollapsed(id: string) {
    this.collapsed[id] = !this.collapsed[id];
    save("collapsed", this.collapsed);
  }

  togglePin(id: string) {
    this.pinned = this.pinned.includes(id) ? this.pinned.filter((p) => p !== id) : [...this.pinned, id];
    this.persist();
  }

  touch(id: string) {
    this.recent = [id, ...this.recent.filter((r) => r !== id)].slice(0, 8);
    this.persist();
  }

  /** Find or create a connection for a quick-connect target. */
  async quick(target: QuickTarget): Promise<Connection> {
    const user = target.user ?? "root";
    const existing = this.list.find((c) => c.host === target.host && c.port === target.port && c.user === user);
    if (existing) return existing;
    const c = blankConnection(null);
    Object.assign(c, {
      name: `${user}@${target.host}`,
      host: target.host,
      port: target.port,
      user,
      auth: { type: "prompt" },
      tags: ["quick"],
    });
    return this.save(c);
  }

  private persist() {
    save("conn-order", this.order);
    save("pinned", this.pinned);
    save("recent", this.recent);
  }
}

export const connections = new Connections();
