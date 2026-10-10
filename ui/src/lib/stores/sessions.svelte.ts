import { api } from "../api";
import type { HostKeyEvent, SessionInfo } from "../api/types";
import { b64ToBytes } from "../util/b64";
import { uuid } from "../util/id";
import {
  findLeaf,
  findLeafBySession,
  leaf,
  leaves,
  mapLeaves,
  removeLeaf,
  setRatio,
  splitLeaf,
  type LayoutNode,
  type SplitDir,
} from "../util/layout";
import { load, save } from "../util/storage";
import { terminals } from "../terminal/host";
import { connections } from "./connections.svelte";

export interface Tab {
  id: string;
  layout: LayoutNode;
  activeLeaf: string;
}

/** Saved workspace: layout whose leaves reference connection ids ("local" for local shells). */
export interface Workspace {
  id: string;
  name: string;
  tabs: LayoutNode[];
}

const COLS = 120;
const ROWS = 32;

export class Sessions {
  infos = $state<Record<string, SessionInfo>>({});
  tabs = $state<Tab[]>([]);
  activeTabId = $state<string | null>(null);
  hostKeys = $state<HostKeyEvent[]>([]);
  /** Label of the last agent that typed into each session (for the "Agent running" banner). */
  lastActor = $state<Record<string, string>>({});
  workspaces = $state<Workspace[]>(load("workspaces", []));

  activeTab = $derived(this.tabs.find((t) => t.id === this.activeTabId) ?? null);
  activeSessionId = $derived.by(() => {
    const tab = this.activeTab;
    if (!tab) return null;
    return (findLeaf(tab.layout, tab.activeLeaf) ?? leaves(tab.layout)[0])?.sessionId ?? null;
  });
  activeSession = $derived(this.activeSessionId ? (this.infos[this.activeSessionId] ?? null) : null);

  private started = false;

  /** Subscribe to backend session events (idempotent). */
  async init() {
    if (this.started) return;
    this.started = true;
    await api.on("session://data", (e) => terminals.get(e.session_id).write(b64ToBytes(e.data_b64)));
    await api.on("session://state", (info) => {
      if (this.infos[info.session_id] || this.tabs.some((t) => findLeafBySession(t.layout, info.session_id)))
        this.infos[info.session_id] = info;
    });
    await api.on("session://agent_mark", (e) => {
      terminals.get(e.session_id).mark(e.command, e.actor_label);
      this.lastActor[e.session_id] = e.actor_label;
    });
    await api.on("session://host_key", (e) => this.hostKeys.push(e));
  }

  /** Replace a disconnected session in place with a fresh one for the same connection. */
  async reconnect(sessionId: string) {
    const old = this.infos[sessionId];
    if (!old) return;
    const info = old.connection_id
      ? await api.openSession(old.connection_id, COLS, ROWS)
      : await api.openLocalSession(null, COLS, ROWS);
    this.infos[info.session_id] = info;
    this.tabs = this.tabs.map((t) => ({
      ...t,
      layout: mapLeaves(t.layout, (l) => (l.sessionId === sessionId ? { ...l, sessionId: info.session_id } : l)),
    }));
    this.dropSession(sessionId);
  }

  async answerHostKey(requestId: string, accept: boolean) {
    this.hostKeys = this.hostKeys.filter((h) => h.request_id !== requestId);
    await api.hostKeyDecision(requestId, accept);
  }

  /** Open a saved connection: new tab, or a split next to the active pane. */
  async open(connectionId: string, split?: SplitDir) {
    connections.touch(connectionId);
    const info = await api.openSession(connectionId, COLS, ROWS);
    this.place(info, split);
    return info;
  }

  async openLocal(shell: string | null = null, split?: SplitDir) {
    const info = await api.openLocalSession(shell, COLS, ROWS);
    this.place(info, split);
    return info;
  }

  /** Put a freshly opened session into the layout. */
  place(info: SessionInfo, split?: SplitDir) {
    this.infos[info.session_id] = info;
    const node = leaf(info.session_id);
    const tab = this.activeTab;
    if (split && tab) {
      this.updateTab(tab.id, (t) => ({ ...t, layout: splitLeaf(t.layout, t.activeLeaf, node, split), activeLeaf: node.id }));
    } else {
      const t: Tab = { id: uuid(), layout: node, activeLeaf: node.id };
      this.tabs.push(t);
      this.activeTabId = t.id;
    }
  }

  private updateTab(id: string, fn: (t: Tab) => Tab) {
    this.tabs = this.tabs.map((t) => (t.id === id ? fn(t) : t));
  }

  focus(tabId: string, leafId?: string) {
    this.activeTabId = tabId;
    if (leafId) this.updateTab(tabId, (t) => ({ ...t, activeLeaf: leafId }));
  }

  /** Focus the pane showing `sessionId` (used by "jump to terminal"). */
  focusSession(sessionId: string) {
    for (const t of this.tabs) {
      const l = findLeafBySession(t.layout, sessionId);
      if (l) return this.focus(t.id, l.id);
    }
  }

  private dropSession(sessionId: string) {
    void api.closeSession(sessionId).catch(() => {});
    terminals.dispose(sessionId);
    delete this.infos[sessionId];
  }

  closeTab(id: string) {
    const idx = this.tabs.findIndex((t) => t.id === id);
    if (idx < 0) return;
    for (const l of leaves(this.tabs[idx].layout)) this.dropSession(l.sessionId);
    this.tabs = this.tabs.filter((t) => t.id !== id);
    if (this.activeTabId === id) this.activeTabId = this.tabs[Math.min(idx, this.tabs.length - 1)]?.id ?? null;
  }

  closePane(tabId: string, leafId: string) {
    const tab = this.tabs.find((t) => t.id === tabId);
    if (!tab) return;
    const l = findLeaf(tab.layout, leafId);
    const rest = removeLeaf(tab.layout, leafId);
    if (!rest) return this.closeTab(tabId);
    if (l) this.dropSession(l.sessionId);
    this.updateTab(tabId, (t) => ({ ...t, layout: rest, activeLeaf: t.activeLeaf === leafId ? leaves(rest)[0].id : t.activeLeaf }));
  }

  /** Drag-to-split: move every pane of `srcTabId` next to `leafId` in `dstTabId`. */
  mergeTab(srcTabId: string, dstTabId: string, leafId: string, dir: SplitDir, before = false) {
    if (srcTabId === dstTabId) return;
    const src = this.tabs.find((t) => t.id === srcTabId);
    const dst = this.tabs.find((t) => t.id === dstTabId);
    if (!src || !dst) return;
    const layout = splitLeaf(dst.layout, leafId, src.layout, dir, before);
    this.tabs = this.tabs.filter((t) => t.id !== srcTabId).map((t) => (t.id === dstTabId ? { ...t, layout, activeLeaf: src.activeLeaf } : t));
    this.activeTabId = dstTabId;
  }

  /** Pull a pane out of its split into its own tab. */
  detachPane(tabId: string, leafId: string) {
    const tab = this.tabs.find((t) => t.id === tabId);
    if (!tab || tab.layout.type === "leaf") return;
    const l = findLeaf(tab.layout, leafId);
    const rest = removeLeaf(tab.layout, leafId);
    if (!l || !rest) return;
    this.updateTab(tabId, (t) => ({ ...t, layout: rest, activeLeaf: leaves(rest)[0].id }));
    const nt: Tab = { id: uuid(), layout: l, activeLeaf: l.id };
    this.tabs.push(nt);
    this.activeTabId = nt.id;
  }

  setRatio(tabId: string, splitId: string, ratio: number) {
    this.updateTab(tabId, (t) => ({ ...t, layout: setRatio(t.layout, splitId, ratio) }));
  }

  cycleTab(delta: number) {
    if (!this.tabs.length) return;
    const i = this.tabs.findIndex((t) => t.id === this.activeTabId);
    this.activeTabId = this.tabs[(i + delta + this.tabs.length) % this.tabs.length].id;
  }

  tabTitle(tab: Tab): string {
    const ls = leaves(tab.layout);
    const first = this.infos[(findLeaf(tab.layout, tab.activeLeaf) ?? ls[0]).sessionId];
    const title = first?.title ?? "…";
    return ls.length > 1 ? `${title} +${ls.length - 1}` : title;
  }

  tabInfo(tab: Tab): SessionInfo | null {
    const l = findLeaf(tab.layout, tab.activeLeaf) ?? leaves(tab.layout)[0];
    return this.infos[l.sessionId] ?? null;
  }

  // ---- workspaces ----
  saveWorkspace(name: string) {
    const tabs = this.tabs.map((t) =>
      mapLeaves(t.layout, (l) => ({ ...l, sessionId: this.infos[l.sessionId]?.connection_id ?? "local" })),
    );
    const ws: Workspace = { id: uuid(), name, tabs: $state.snapshot(tabs) as LayoutNode[] };
    this.workspaces = [...this.workspaces.filter((w) => w.name !== name), ws];
    save("workspaces", this.workspaces);
    return ws;
  }

  deleteWorkspace(id: string) {
    this.workspaces = this.workspaces.filter((w) => w.id !== id);
    save("workspaces", this.workspaces);
  }

  async openWorkspace(id: string) {
    const ws = this.workspaces.find((w) => w.id === id);
    if (!ws) return;
    for (const layout of ws.tabs) {
      const mapping = new Map<string, string>();
      for (const l of leaves(layout)) {
        const ref = l.sessionId;
        const info =
          ref === "local" || !connections.byId.has(ref)
            ? await api.openLocalSession(null, COLS, ROWS)
            : await api.openSession(ref, COLS, ROWS);
        this.infos[info.session_id] = info;
        mapping.set(l.id, info.session_id);
      }
      const fresh = mapLeaves(layout, (l) => ({ ...leaf(mapping.get(l.id)!) }));
      const t: Tab = { id: uuid(), layout: fresh, activeLeaf: leaves(fresh)[0].id };
      this.tabs.push(t);
      this.activeTabId = t.id;
    }
  }
}

export const sessions = new Sessions();
