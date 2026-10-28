import { api } from "../api";
import type { AuthPromptEvent, HostKeyEvent, SessionInfo, TunnelStatus } from "../api/types";
import { errorMessage } from "../api/errors";
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
import { agent } from "./agent.svelte";
import { editors } from "./editors.svelte";
import { ui } from "./ui.svelte";
import { RECONNECT_ATTEMPTS, reconnectDelay, retryable, shouldAutoReconnect } from "../util/reconnect";
import { t } from "../i18n";
import { s3 } from "./s3.svelte";

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

/** A dropped SSH session logging in again in the background, keyed by the old session id. */
export interface Reconnect {
  /** Attempts made so far. */
  attempt: number;
  /** When the next attempt starts (ms epoch); null while one runs. */
  nextAt: number | null;
  /** Why the connection dropped, then why the last attempt failed. */
  error: string | null;
  /** False once it gave up or was stopped. */
  running: boolean;
}

/** A pane whose SSH connection is still logging in. Its layout leaf holds `pending:<ref>`. */
export interface PendingConnection {
  ref: string;
  connectionId: string;
  label: string;
  /** user@host, for "Connecting to …". */
  target: string;
  /** A login question waiting for the user. */
  prompt: AuthPromptEvent | null;
  error: string | null;
}

const PENDING = "pending:";
export const isPending = (sessionId: string) => sessionId.startsWith(PENDING);
export const pendingRef = (sessionId: string) => sessionId.slice(PENDING.length);

/** A file-browser pane holds `files:<ssh session id>` in its layout leaf. */
const FILES = "files:";
export const isFiles = (leafSession: string) => leafSession.startsWith(FILES);
export const filesSession = (leafSession: string) => leafSession.slice(FILES.length);
/** An S3 browser pane holds `s3:<account id>`. */
const S3 = "s3:";
export const isS3 = (leafSession: string) => leafSession.startsWith(S3);
export const s3AccountOf = (leafSession: string) => leafSession.slice(S3.length);
/** A file editor pane holds `edit:<doc id>` (see the editors store). */
/** A remote desktop pane: `rdp:<connection id>#<instance>` (one connection can be open twice). */
const RDP = "rdp:";
export const isRdp = (leafSession: string) => leafSession.startsWith(RDP);
export const rdpConnectionOf = (leafSession: string) => leafSession.slice(RDP.length).split("#")[0];
const rdpLeaf = (connectionId: string) => `${RDP}${connectionId}#${uuid()}`;

const EDIT = "edit:";
export const isEdit = (leafSession: string) => leafSession.startsWith(EDIT);
export const editDocOf = (leafSession: string) => leafSession.slice(EDIT.length);
/** Whether a leaf shows a live terminal (not a connecting pane, file browser or editor). */
const isTerminal = (leafSession: string) =>
  !isPending(leafSession) && !isFiles(leafSession) && !isS3(leafSession) && !isEdit(leafSession) && !isRdp(leafSession);

export class Sessions {
  infos = $state<Record<string, SessionInfo>>({});
  /** The user and host of each session's own login shell (its first integrated prompt): a
   *  different host later means the user is in a shell on another machine (`ssh` inside it). */
  homes: Record<string, { user: string | null; host: string }> = {};
  tabs = $state<Tab[]>([]);
  activeTabId = $state<string | null>(null);
  hostKeys = $state<HostKeyEvent[]>([]);
  pending = $state<Record<string, PendingConnection>>({});
  reconnects = $state<Record<string, Reconnect>>({});
  /** Cuts the wait before a reconnect attempt short. */
  private wakers = new Map<string, () => void>();
  /** Label of the last agent that typed into each session (for the "Agent running" banner). */
  lastActor = $state<Record<string, string>>({});
  /** Tabs where typing goes to every terminal pane ("type in all panes"). */
  broadcast = $state<Record<string, boolean>>({});
  /** Port forwards of SSH sessions whose connection has any. */
  tunnels = $state<Record<string, TunnelStatus[]>>({});
  workspaces = $state<Workspace[]>(load("workspaces", []));

  activeTab = $derived(this.tabs.find((t) => t.id === this.activeTabId) ?? null);
  activeSessionId = $derived.by(() => {
    const tab = this.activeTab;
    if (!tab) return null;
    const id = (findLeaf(tab.layout, tab.activeLeaf) ?? leaves(tab.layout)[0])?.sessionId ?? null;
    return id && isTerminal(id) ? id : null;
  });
  activeSession = $derived(this.activeSessionId ? (this.infos[this.activeSessionId] ?? null) : null);
  /** The SSH session the focused pane works on: its terminal, file browser or file editor. */
  contextSession = $derived.by(() => {
    const tab = this.activeTab;
    const id = tab ? ((findLeaf(tab.layout, tab.activeLeaf) ?? leaves(tab.layout)[0])?.sessionId ?? null) : null;
    if (!id) return null;
    const sid = isFiles(id) ? filesSession(id) : isEdit(id) ? (editors.docs[editDocOf(id)]?.fs.sessionId ?? null) : isTerminal(id) ? id : null;
    return sid ? (this.infos[sid] ?? null) : null;
  });

  private started = false;

  /** Subscribe to backend session events (idempotent). */
  async init() {
    if (this.started) return;
    this.started = true;
    await api.on("session://data", (e) => terminals.get(e.session_id).write(b64ToBytes(e.data_b64)));
    terminals.input = (sessionId, data) => {
      for (const target of this.inputTargets(sessionId)) terminals.write(target, data);
    };
    await api.on("session://state", (info) => {
      if (!this.infos[info.session_id] && !this.tabs.some((t) => findLeafBySession(t.layout, info.session_id))) return;
      this.infos[info.session_id] = info;
      if (info.integration === "full" && info.host && !this.homes[info.session_id]) this.homes[info.session_id] = { user: info.user, host: info.host };
      const auth = info.connection_id ? connections.byId.get(info.connection_id)?.auth.type : undefined;
      if (shouldAutoReconnect(info, auth) && !this.reconnects[info.session_id]) void this.autoReconnect(info.session_id, false);
    });
    // Back online (wake from sleep, Wi-Fi again): try at once instead of waiting out the backoff.
    if (typeof window !== "undefined") window.addEventListener("online", () => this.wakers.forEach((wake) => wake()));
    await api.on("session://agent_mark", (e) => {
      terminals.get(e.session_id).mark(e.command, e.actor_label);
      this.lastActor[e.session_id] = e.actor_label;
    });
    await api.on("session://host_key", (e) => this.hostKeys.push(e));
    await api.on("session://tunnels", (e) => {
      if (this.infos[e.session_id]) this.tunnels[e.session_id] = e.tunnels;
    });
    // An agent proposed a file edit: show the diff in the file's editor.
    await api.on("edit://proposal", (p) => this.openEditor(editors.propose(p)));
    await api.on("edit://resolved", (id) => editors.resolved(id));
    await api.on("session://auth_prompt", (e) => {
      const p = this.pending[e.prompt_ref];
      if (p) this.pending[e.prompt_ref] = { ...p, prompt: e };
      else void api.answerAuthPrompt(e.request_id, null);
    });
  }

  /** Log in to `connectionId` into the pane holding `pending:<ref>`, then swap in the session. */
  private async connect(ref: string, connectionId: string): Promise<SessionInfo | null> {
    const c = connections.byId.get(connectionId);
    this.pending[ref] = {
      ref,
      connectionId,
      label: c?.name ?? connectionId,
      target: c ? `${c.user}@${c.host}` : connectionId,
      prompt: null,
      error: null,
    };
    try {
      const info = await api.openSession(connectionId, COLS, ROWS, ref);
      if (!this.pending[ref]) {
        // The pane was closed while logging in.
        void api.closeSession(info.session_id).catch(() => {});
        return null;
      }
      this.infos[info.session_id] = info;
      this.swapSession(PENDING + ref, info.session_id);
      delete this.pending[ref];
      void this.loadTunnels(info.session_id);
      return info;
    } catch (e) {
      const p = this.pending[ref];
      if (p) this.pending[ref] = { ...p, prompt: null, error: errorMessage(e) };
      return null;
    }
  }

  /** Try a failed login again in the same pane. */
  retry(ref: string) {
    const p = this.pending[ref];
    if (!p) return;
    // Made a remote desktop since (the pane's port 3389 hint): the pane becomes one.
    if (connections.byId.get(p.connectionId)?.kind === "rdp") {
      this.swapSession(PENDING + ref, rdpLeaf(p.connectionId));
      delete this.pending[ref];
      return;
    }
    void this.connect(ref, p.connectionId);
  }

  /** Answer the pane's login prompt (`null` cancels the login). */
  async answerAuth(ref: string, answers: string[] | null) {
    const p = this.pending[ref];
    if (!p?.prompt) return;
    const id = p.prompt.request_id;
    this.pending[ref] = { ...p, prompt: null };
    await api.answerAuthPrompt(id, answers);
  }

  private swapSession(from: string, to: string) {
    this.tabs = this.tabs.map((t) => ({
      ...t,
      layout: mapLeaves(t.layout, (l) => (l.sessionId === from ? { ...l, sessionId: to } : l)),
    }));
  }

  /**
   * Log a dropped SSH session in again, keeping its terminal: retries with backoff while the
   * network is the problem, stops on a login or host key error (or a login prompt).
   */
  private async autoReconnect(sessionId: string, now: boolean) {
    const connectionId = this.infos[sessionId]?.connection_id;
    if (!connectionId) return;
    this.reconnects[sessionId] = { attempt: 0, nextAt: null, error: this.infos[sessionId].ended?.error ?? null, running: true };
    for (let n = 0; n < RECONNECT_ATTEMPTS; n++) {
      // Automatic: a first pause lets a flapping network settle. Manual: try right away.
      const wait = reconnectDelay(now ? n : n + 1);
      let r = this.reconnects[sessionId];
      if (!r?.running) return;
      if (wait) {
        r.nextAt = Date.now() + wait;
        await this.pause(sessionId, wait);
      }
      r = this.reconnects[sessionId];
      if (!r?.running) return;
      r.nextAt = null;
      r.attempt = n + 1;
      const term = terminals.peek(sessionId)?.term;
      try {
        // A ref no pane knows: a login prompt is declined, so this never asks the user.
        const next = await api.openSession(connectionId, term?.cols ?? COLS, term?.rows ?? ROWS, uuid());
        if (!this.reconnects[sessionId]?.running || !this.infos[sessionId]) {
          void api.closeSession(next.session_id).catch(() => {});
          return;
        }
        this.adopt(sessionId, next);
        return;
      } catch (e) {
        const cur = this.reconnects[sessionId];
        if (!cur) return;
        cur.error = errorMessage(e);
        if (!retryable(e)) break;
      }
    }
    const r = this.reconnects[sessionId];
    if (r) Object.assign(r, { running: false, nextAt: null });
  }

  private pause(sessionId: string, ms: number) {
    return new Promise<void>((resolve) => {
      const done = () => {
        clearTimeout(timer);
        this.wakers.delete(sessionId);
        resolve();
      };
      const timer = setTimeout(done, ms);
      this.wakers.set(sessionId, done);
    });
  }

  /** Move the terminal of dropped session `old` (and its file browser) to the new session. */
  private adopt(old: string, next: SessionInfo) {
    terminals.peek(old)?.write(new TextEncoder().encode(`\r\n\x1b[2m── ${t("term.reconnectedLine")} ──\x1b[0m\r\n`));
    terminals.rekey(old, next.session_id);
    this.infos[next.session_id] = next;
    void this.loadTunnels(next.session_id);
    delete this.tunnels[old];
    this.swapSession(old, next.session_id);
    this.swapSession(FILES + old, FILES + next.session_id);
    agent.moveSession(old, next.session_id);
    if (this.lastActor[old]) this.lastActor[next.session_id] = this.lastActor[old];
    delete this.lastActor[old];
    delete this.reconnects[old];
    void api.closeSession(old).catch(() => {});
    delete this.infos[old];
  }

  /** The session's tunnels as they stand (the "session://tunnels" event brings the outcome). */
  private async loadTunnels(sessionId: string) {
    const list = await api.sessionTunnels(sessionId).catch(() => []);
    // An event may have brought the started tunnels meanwhile.
    if (list.length && this.infos[sessionId] && !this.tunnels[sessionId]?.some((x) => x.port || x.error)) this.tunnels[sessionId] = list;
  }

  /**
   * The live terminals typing in a tab reaches while "type in all panes" is on: every terminal
   * pane except those an agent is driving or that are disconnected.
   */
  broadcastTargets(tabId: string): string[] {
    const tab = this.tabs.find((x) => x.id === tabId);
    if (!tab) return [];
    return leaves(tab.layout)
      .map((l) => l.sessionId)
      .filter((id) => isTerminal(id) && this.infos[id] && this.infos[id].state !== "disconnected" && this.infos[id].state !== "agent_running");
  }

  /** Whether typing in `tabId` currently goes to more than one terminal. */
  broadcasting(tabId: string): boolean {
    return !!this.broadcast[tabId] && this.broadcastTargets(tabId).length > 1;
  }

  /** Where input typed in `sessionId` goes: that session, or every pane of its tab. */
  inputTargets(sessionId: string): string[] {
    const tab = this.tabs.find((x) => this.broadcast[x.id] && findLeafBySession(x.layout, sessionId));
    if (!tab) return [sessionId];
    const targets = this.broadcastTargets(tab.id);
    return targets.includes(sessionId) ? targets : [sessionId, ...targets];
  }

  toggleBroadcast(tabId: string) {
    if (this.broadcast[tabId]) delete this.broadcast[tabId];
    else this.broadcast[tabId] = true;
  }

  /** Try now: skips the wait, or starts again after giving up. */
  reconnectNow(sessionId: string) {
    if (this.reconnects[sessionId]?.running) this.wakers.get(sessionId)?.();
    else void this.autoReconnect(sessionId, true);
  }

  stopReconnect(sessionId: string) {
    const r = this.reconnects[sessionId];
    if (r) Object.assign(r, { running: false, nextAt: null });
    this.wakers.get(sessionId)?.();
  }

  /** Replace a disconnected session with a fresh one for the same connection. */
  async reconnect(sessionId: string) {
    const old = this.infos[sessionId];
    if (!old) return;
    const auth = old.connection_id ? connections.byId.get(old.connection_id)?.auth.type : undefined;
    // Saved credentials: log in again in place, keeping the terminal. Otherwise the login pane asks.
    if (auth && auth !== "prompt") return this.reconnectNow(sessionId);
    if (old.connection_id) {
      const ref = uuid();
      this.swapSession(sessionId, PENDING + ref);
      this.dropSession(sessionId);
      await this.connect(ref, old.connection_id);
      return;
    }
    const info = await api.openLocalSession(null, COLS, ROWS);
    this.infos[info.session_id] = info;
    this.swapSession(sessionId, info.session_id);
    this.dropSession(sessionId);
  }

  async answerHostKey(requestId: string, accept: boolean) {
    this.hostKeys = this.hostKeys.filter((h) => h.request_id !== requestId);
    await api.hostKeyDecision(requestId, accept);
  }

  /** Open a saved connection: new tab, or a split next to the active pane. */
  async open(connectionId: string, split?: SplitDir) {
    connections.touch(connectionId);
    // A remote desktop pane connects itself once it knows its size.
    if (connections.byId.get(connectionId)?.kind === "rdp") return void this.placeLeaf(rdpLeaf(connectionId), split);
    const ref = uuid();
    this.placeLeaf(PENDING + ref, split);
    return this.connect(ref, connectionId);
  }

  async openLocal(shell: string | null = null, split?: SplitDir) {
    const info = await api.openLocalSession(shell, COLS, ROWS);
    this.place(info, split);
    return info;
  }

  /** Put a freshly opened session into the layout. */
  place(info: SessionInfo, split?: SplitDir) {
    this.infos[info.session_id] = info;
    this.placeLeaf(info.session_id, split);
  }

  private placeLeaf(sessionId: string, split?: SplitDir) {
    const node = leaf(sessionId);
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

  /** Open the file browser for an SSH session, split to the right of its terminal. */
  openFiles(sessionId: string) {
    const leafId = FILES + sessionId;
    for (const t of this.tabs) {
      const existing = findLeafBySession(t.layout, leafId);
      if (existing) return this.focus(t.id, existing.id);
    }
    for (const t of this.tabs) {
      const term = findLeafBySession(t.layout, sessionId);
      if (!term) continue;
      const node = leaf(leafId);
      this.updateTab(t.id, (tab) => ({ ...tab, layout: splitLeaf(tab.layout, term.id, node, "row"), activeLeaf: node.id }));
      this.activeTabId = t.id;
      return;
    }
  }

  /**
   * Show editor doc `docId`: focus its pane, else split it to the right of its session's file
   * browser (or terminal) when that tab has a single pane, else open a new tab (an editor
   * squeezed into a third column is too narrow to use).
   */
  openEditor(docId: string) {
    const leafSession = EDIT + docId;
    for (const t of this.tabs) {
      const existing = findLeafBySession(t.layout, leafSession);
      if (existing) return this.focus(t.id, existing.id);
    }
    const sid = editors.docs[docId]?.fs.sessionId;
    for (const near of sid ? [FILES + sid, sid] : [])
      for (const t of this.tabs) {
        const at = findLeafBySession(t.layout, near);
        if (!at) continue;
        if (leaves(t.layout).length > 1) {
          this.placeLeaf(leafSession);
          return;
        }
        const node = leaf(leafSession);
        this.updateTab(t.id, (tab) => ({ ...tab, layout: splitLeaf(tab.layout, at.id, node, "row"), activeLeaf: node.id }));
        this.activeTabId = t.id;
        return;
      }
    this.placeLeaf(leafSession);
  }

  /** Run `close` now, or after the user agrees to drop unsaved edits in `leafSessions`. */
  private closeChecked(leafSessions: string[], close: () => void) {
    const dirty = leafSessions.filter((l) => isEdit(l) && editors.dirty(editDocOf(l)));
    if (!dirty.length) return close();
    void (async () => {
      for (const l of dirty) {
        const name = editors.docs[editDocOf(l)]?.name ?? "";
        if (!(await ui.ask(t("edit.discardTitle", { name }), { body: t("edit.discardBody"), confirmLabel: t("edit.discard"), danger: true }))) return;
      }
      close();
    })();
  }

  /** Open an S3 account's browser: focus it if open, else a new tab (or a split). */
  openS3(accountId: string, split?: SplitDir) {
    const leafSession = S3 + accountId;
    for (const t of this.tabs) {
      const existing = findLeafBySession(t.layout, leafSession);
      if (existing) return this.focus(t.id, existing.id);
    }
    this.placeLeaf(leafSession, split);
  }

  private dropSession(sessionId: string) {
    if (isEdit(sessionId)) return editors.close(editDocOf(sessionId));
    // Remote desktops close with their pane (RdpView).
    if (isFiles(sessionId) || isS3(sessionId) || isRdp(sessionId)) return;
    if (isPending(sessionId)) {
      const ref = pendingRef(sessionId);
      const p = this.pending[ref];
      delete this.pending[ref];
      if (p?.prompt) void api.answerAuthPrompt(p.prompt.request_id, null);
      return;
    }
    this.stopReconnect(sessionId);
    delete this.reconnects[sessionId];
    void api.closeSession(sessionId).catch(() => {});
    terminals.dispose(sessionId);
    delete this.infos[sessionId];
    delete this.tunnels[sessionId];
  }

  closeTab(id: string) {
    const tab = this.tabs.find((t) => t.id === id);
    if (tab) this.closeChecked(leaves(tab.layout).map((l) => l.sessionId), () => this.removeTab(id));
  }

  private removeTab(id: string) {
    const idx = this.tabs.findIndex((t) => t.id === id);
    if (idx < 0) return;
    for (const l of leaves(this.tabs[idx].layout)) this.dropSession(l.sessionId);
    this.tabs = this.tabs.filter((t) => t.id !== id);
    delete this.broadcast[id];
    if (this.activeTabId === id) this.activeTabId = this.tabs[Math.min(idx, this.tabs.length - 1)]?.id ?? null;
  }

  closePane(tabId: string, leafId: string) {
    const tab = this.tabs.find((t) => t.id === tabId);
    const target = tab && findLeaf(tab.layout, leafId);
    if (target) this.closeChecked([target.sessionId], () => this.removePane(tabId, leafId));
  }

  private removePane(tabId: string, leafId: string) {
    const tab = this.tabs.find((t) => t.id === tabId);
    if (!tab) return;
    const l = findLeaf(tab.layout, leafId);
    const rest = removeLeaf(tab.layout, leafId);
    if (!rest) return this.removeTab(tabId);
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
    delete this.broadcast[srcTabId];
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
    const id = (findLeaf(tab.layout, tab.activeLeaf) ?? ls[0]).sessionId;
    const title = this.paneTitle(id);
    return ls.length > 1 ? `${title} +${ls.length - 1}` : title;
  }

  /** Title of the pane holding `leafSession`. */
  paneTitle(leafSession: string): string {
    if (isPending(leafSession)) return this.pending[pendingRef(leafSession)]?.label ?? "…";
    if (isFiles(leafSession)) return `Files · ${this.infos[filesSession(leafSession)]?.title ?? "…"}`;
    if (isS3(leafSession)) return `S3 · ${s3.byId(s3AccountOf(leafSession))?.name ?? "…"}`;
    if (isRdp(leafSession)) return connections.byId.get(rdpConnectionOf(leafSession))?.name ?? "…";
    if (isEdit(leafSession)) {
      const id = editDocOf(leafSession);
      return `${editors.docs[id]?.name ?? "…"}${editors.dirty(id) ? " •" : ""}`;
    }
    return this.infos[leafSession]?.title ?? "…";
  }

  tabInfo(tab: Tab): SessionInfo | null {
    const l = findLeaf(tab.layout, tab.activeLeaf) ?? leaves(tab.layout)[0];
    return this.infos[l.sessionId] ?? null;
  }

  // ---- workspaces ----
  saveWorkspace(name: string) {
    // File browsers are not saved; a pane still logging in is saved as its connection.
    const browser = (id: string) => isFiles(id) || isS3(id) || isEdit(id);
    const connOf = (id: string) =>
      (isRdp(id) ? rdpConnectionOf(id) : isPending(id) ? this.pending[pendingRef(id)]?.connectionId : this.infos[id]?.connection_id) ?? "local";
    const tabs = this.tabs.flatMap((t) => {
      let layout: LayoutNode | null = t.layout;
      for (const l of leaves(t.layout)) if (layout && browser(l.sessionId)) layout = removeLeaf(layout, l.id);
      return layout ? [mapLeaves(layout, (l) => ({ ...l, sessionId: connOf(l.sessionId) }))] : [];
    });
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
      const logins: [string, string][] = [];
      for (const l of leaves(layout)) {
        const conn = l.sessionId;
        if (connections.byId.get(conn)?.kind === "rdp") {
          mapping.set(l.id, rdpLeaf(conn));
        } else if (conn === "local" || !connections.byId.has(conn)) {
          const info = await api.openLocalSession(null, COLS, ROWS);
          this.infos[info.session_id] = info;
          mapping.set(l.id, info.session_id);
        } else {
          const ref = uuid();
          logins.push([ref, conn]);
          mapping.set(l.id, PENDING + ref);
        }
      }
      const fresh = mapLeaves(layout, (l) => ({ ...leaf(mapping.get(l.id)!) }));
      const t: Tab = { id: uuid(), layout: fresh, activeLeaf: leaves(fresh)[0].id };
      this.tabs.push(t);
      this.activeTabId = t.id;
      // Each pane logs in on its own (and may ask for a password).
      for (const [ref, conn] of logins) void this.connect(ref, conn);
    }
  }
}

export const sessions = new Sessions();
