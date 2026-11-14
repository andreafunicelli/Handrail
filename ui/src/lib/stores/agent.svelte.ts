import { api } from "../api";
import type {
  AgentEvent,
  ApprovalDecision,
  ApprovalRequest,
  Brain,
  DetectedHarness,
  HarnessProfile,
  Provider,
} from "../api/types";
import { load, save } from "../util/storage";
import { errorMessage, toasts } from "./toasts.svelte";
import { opsLog } from "./opsLog.svelte";

/** Every item carries `at`: when it was created (epoch ms), for the Activity timeline. */
export type ChatItem =
  | { kind: "user"; id: string; at: number; text: string }
  | { kind: "assistant"; id: string; at: number; text: string; thinking: string }
  | {
      kind: "tool";
      id: string;
      at: number;
      callId: string;
      name: string;
      input: Record<string, unknown>;
      output: unknown;
      isError: boolean;
      done: boolean;
      /** When the result arrived. */
      doneAt: number | null;
      /** The run ended before the tool returned a result. */
      aborted: boolean;
      /** The brain that made the call (its label when the message was sent). */
      actor: string | null;
    }
  | {
      kind: "approval";
      id: string;
      at: number;
      req: ApprovalRequest;
      decision: ApprovalDecision | null;
      decidedAt: number | null;
      /** The run_command call this approval belongs to, when it could be matched. */
      callId: string | null;
      /** The backend withdrew the request unanswered (timeout, cancelled run): it denied it. */
      expired: boolean;
    }
  | { kind: "error"; id: string; at: number; message: string; retryable: boolean }
  | { kind: "limit"; id: string; at: number; steps: number };

export type ToolItem = Extract<ChatItem, { kind: "tool" }>;
export type ApprovalItem = Extract<ChatItem, { kind: "approval" }>;

export interface Chat {
  items: ChatItem[];
  runId: string | null;
  usage: { input: number; output: number; cost: number | null };
  lastMessage: string | null;
  /** Label of the brain the last message went to. */
  actor: string | null;
  /** The user stopped the latest run. */
  cancelled: boolean;
  /** A message was sent and its run id is not known yet: no second run may start. */
  starting: boolean;
}

/** Whether the chat has a run going (or one being started). */
export const chatBusy = (c: Chat | undefined) => !!(c?.runId || c?.starting);

export interface BrainOption {
  key: string;
  brain: Brain;
  label: string;
  detail: string;
  harness: HarnessProfile | null;
}

let seq = 0;
const nid = () => `i${++seq}`;

class Agent {
  chats = $state<Record<string, Chat>>({});
  providers = $state<Provider[]>([]);
  profiles = $state<HarnessProfile[]>([]);
  detected = $state<DetectedHarness[]>([]);
  /** Brain per session; falls back to the last brain the user picked. */
  brainBySession = $state<Record<string, string>>({});
  defaultBrain = $state<string | null>(load("brain", null));
  /** Model picked per provider in the brain picker (defaults to the provider's default model). */
  models = $state<Record<string, string>>(load("brain-models", {}));

  options = $derived.by<BrainOption[]>(() => {
    const out: BrainOption[] = [];
    for (const p of this.providers) {
      const model = this.models[p.id] || p.default_model;
      out.push({
        key: `b:${p.id}`,
        brain: { type: "built_in", provider: p.id, model },
        label: p.name,
        detail: model,
        harness: null,
      });
    }
    for (const h of this.profiles)
      out.push({
        key: `h:${h.id}`,
        brain: { type: "harness", profile: h.id },
        label: h.name,
        detail: h.harness,
        harness: h,
      });
    return out;
  });

  /** Pending approvals of every session, oldest first (so none is lost behind another tab). */
  allPending = $derived.by(() => {
    const out: { sessionId: string; item: ApprovalItem }[] = [];
    for (const [sessionId, chat] of Object.entries(this.chats))
      for (const i of chat.items) if (i.kind === "approval" && !i.decision) out.push({ sessionId, item: i });
    return out.sort((a, b) => a.item.at - b.item.at);
  });

  private started = false;
  /** Runs that ended or were stopped: their late events are not this chat's any more. */
  private over = new Set<string>();
  /** Sessions closed: a late event must not bring their chat back. */
  private closed = new Set<string>();

  async init() {
    if (this.started) return;
    this.started = true;
    await api.on("agent://event", (e) => this.apply(e.session_id, e.run_id, e.event));
    await api.on("approval://requested", (req) => this.addApproval(req));
    await api.on("approval://resolved", (id) => this.expire(id));
    void opsLog.init();
    await this.reload();
  }

  async reload() {
    try {
      const [providers, profiles] = await Promise.all([api.listProviders(), api.listHarnessProfiles()]);
      this.providers = providers;
      this.profiles = profiles;
    } catch (e) {
      toasts.error(e);
    }
  }

  async detect() {
    this.detected = await api.detectHarnesses();
  }

  chat(sessionId: string): Chat {
    let c = this.chats[sessionId];
    if (!c) {
      this.chats[sessionId] = {
        items: [],
        runId: null,
        usage: { input: 0, output: 0, cost: null },
        lastMessage: null,
        actor: null,
        cancelled: false,
        starting: false,
      };
      c = this.chats[sessionId];
    }
    return c;
  }

  brainFor(sessionId: string | null): BrainOption | null {
    const key = (sessionId && this.brainBySession[sessionId]) || this.defaultBrain;
    return this.options.find((o) => o.key === key) ?? this.options[0] ?? null;
  }

  setBrain(sessionId: string | null, key: string) {
    if (sessionId) this.brainBySession[sessionId] = key;
    this.defaultBrain = key;
    save("brain", key);
  }

  setModel(providerId: string, model: string) {
    this.models[providerId] = model;
    save("brain-models", this.models);
  }

  pendingApprovals(sessionId: string) {
    return (this.chats[sessionId]?.items ?? []).filter((i): i is ApprovalItem => i.kind === "approval" && !i.decision);
  }

  /** `display`: what the chat shows for the message when it carries context (file contents). */
  async send(sessionId: string, message: string, display?: string) {
    const brain = this.brainFor(sessionId);
    if (!brain) return;
    const chat = this.chat(sessionId);
    // Set before the first await: a second click or Enter sees the run as started.
    if (chatBusy(chat)) return;
    chat.starting = true;
    chat.items.push({ kind: "user", id: nid(), at: Date.now(), text: display ?? message });
    chat.lastMessage = message;
    chat.actor = brain.harness ? brain.label : `${brain.label} · ${brain.detail}`;
    chat.cancelled = false;
    try {
      const { run_id } = await api.agentSend(sessionId, $state.snapshot(brain.brain) as Brain, message);
      // Stopped while starting: stop the run now that it has an id.
      if (chat.cancelled) {
        this.over.add(run_id);
        if (chat.runId === run_id) chat.runId = null;
        void api.agentCancel(run_id).catch(() => {});
      } else if (!this.over.has(run_id)) chat.runId = run_id; // its "done" may have come first
    } catch (e) {
      chat.items.push({ kind: "error", id: nid(), at: Date.now(), message: errorMessage(e), retryable: true });
    } finally {
      chat.starting = false;
    }
  }

  async cancel(sessionId: string) {
    const chat = this.chats[sessionId];
    if (!chat || !chatBusy(chat)) return;
    chat.cancelled = true;
    // Still starting with no id yet: `send` stops the run once it has one.
    const id = chat.runId;
    if (!id) return;
    this.over.add(id);
    chat.runId = null;
    await api.agentCancel(id).catch(() => {});
  }

  async decide(sessionId: string, approvalId: string, decision: ApprovalDecision) {
    const item = this.chats[sessionId]?.items.find((i): i is ApprovalItem => i.kind === "approval" && i.req.approval_id === approvalId);
    if (item) {
      item.decision = decision;
      item.decidedAt = Date.now();
    }
    try {
      await api.approvalDecide(approvalId, decision);
    } catch (e) {
      if (item) {
        item.decision = null;
        item.decidedAt = null;
      }
      toasts.error(e);
    }
  }

  /** A reconnect replaced session `from` with `to`: the conversation and brain choice follow. */
  moveSession(from: string, to: string) {
    const chat = this.chats[from];
    if (chat) {
      if (chat.runId) this.over.add(chat.runId);
      this.chats[to] = { ...chat, runId: null, starting: false };
      delete this.chats[from];
    }
    if (this.brainBySession[from]) {
      this.brainBySession[to] = this.brainBySession[from];
      delete this.brainBySession[from];
    }
  }

  clear(sessionId: string) {
    if (chatBusy(this.chats[sessionId])) return;
    delete this.chats[sessionId];
  }

  /** The session closed: its run is stopped, approvals still waiting are denied, and the
   *  conversation goes (nothing can answer or jump to it any more). */
  release(sessionId: string) {
    const chat = this.chats[sessionId];
    if (chat) {
      chat.cancelled = true;
      if (chat.runId) {
        this.over.add(chat.runId);
        void api.agentCancel(chat.runId).catch(() => {});
      }
      for (const a of this.pendingApprovals(sessionId)) void api.approvalDecide(a.req.approval_id, { type: "deny", message: null }).catch(() => {});
    }
    delete this.chats[sessionId];
    delete this.brainBySession[sessionId];
    this.closed.add(sessionId);
  }

  private addApproval(req: ApprovalRequest) {
    if (this.closed.has(req.session_id)) return;
    const chat = this.chat(req.session_id);
    if (chat.items.some((i) => i.kind === "approval" && i.req.approval_id === req.approval_id)) return;
    chat.items.push({
      kind: "approval",
      id: nid(),
      at: Date.now(),
      req,
      decision: null,
      decidedAt: null,
      callId: matchCall(chat.items, req.command),
      expired: false,
    });
  }

  /** The backend dropped a request (no answer in time, run cancelled): it counts as denied. */
  private expire(approvalId: string) {
    for (const chat of Object.values(this.chats)) {
      const a = chat.items.find((i): i is ApprovalItem => i.kind === "approval" && i.req.approval_id === approvalId);
      if (!a) continue;
      if (!a.decision) {
        a.decision = { type: "deny", message: null };
        a.decidedAt = Date.now();
        a.expired = true;
      }
      return;
    }
  }

  /** Record a finished tool call in the operations history (batched; never on the hot path). */
  private log(sessionId: string, chat: Chat, tool: ToolItem) {
    const approval = chat.items.find((i): i is ApprovalItem => i.kind === "approval" && i.callId === tool.callId) ?? null;
    opsLog.record({
      sessionId,
      tool: tool.name,
      input: $state.snapshot(tool.input) as Record<string, unknown>,
      output: $state.snapshot(tool.output),
      isError: tool.isError,
      aborted: tool.aborted,
      actor: approval?.req.actor_label ?? tool.actor,
      approval: approval ? { decision: $state.snapshot(approval.decision) as ApprovalDecision | null, risk: approval.req.risk } : null,
      startedAt: tool.at,
      endedAt: tool.doneAt ?? Date.now(),
    });
  }

  /** Fold one normalized AgentEvent into the session's transcript. */
  apply(sessionId: string, runId: string, ev: AgentEvent) {
    if (this.closed.has(sessionId)) return;
    // Late events of a run that ended or was stopped, or of an older run while a newer one
    // goes: not part of what is on screen now.
    const known = this.chats[sessionId];
    if (this.over.has(runId) || (known?.runId && known.runId !== runId)) {
      // A stopped run's end still closes its open tool cards, unless a newer run is under way.
      if (ev.type === "done" && known && !chatBusy(known)) this.abortOpenTools(sessionId, known);
      return;
    }
    const chat = this.chat(sessionId);
    if (ev.type !== "done" && !chat.runId) chat.runId = runId;
    const last = chat.items.at(-1);
    switch (ev.type) {
      case "text_delta":
      case "thinking_delta": {
        let a = last?.kind === "assistant" ? last : null;
        if (!a) {
          chat.items.push({ kind: "assistant", id: nid(), at: Date.now(), text: "", thinking: "" });
          a = chat.items.at(-1) as Extract<ChatItem, { kind: "assistant" }>;
        }
        if (ev.type === "text_delta") a.text += ev.text;
        else a.thinking += ev.text;
        break;
      }
      case "tool_call": {
        const input = (ev.input && typeof ev.input === "object" ? ev.input : {}) as Record<string, unknown>;
        chat.items.push({
          kind: "tool",
          id: nid(),
          at: Date.now(),
          callId: ev.call_id,
          name: ev.name,
          input,
          output: null,
          isError: false,
          done: false,
          doneAt: null,
          aborted: false,
          actor: chat.actor,
        });
        // The approval can arrive on its own channel before the call: link it now.
        if (ev.name === "run_command" && typeof input.command === "string") {
          const orphan = chat.items.find(
            (i): i is ApprovalItem => i.kind === "approval" && !i.callId && !i.decision && i.req.command === input.command,
          );
          if (orphan) orphan.callId = ev.call_id;
        }
        break;
      }
      case "tool_result": {
        const t = chat.items.find((i): i is ToolItem => i.kind === "tool" && i.callId === ev.call_id);
        if (t && !t.done) {
          t.output = ev.output;
          t.isError = ev.is_error;
          t.done = true;
          t.doneAt = Date.now();
          this.log(sessionId, chat, t);
        }
        break;
      }
      case "approval_requested": {
        const { type: _type, ...req } = ev;
        this.addApproval(req);
        break;
      }
      case "approval_resolved": {
        const a = chat.items.find((i): i is ApprovalItem => i.kind === "approval" && i.req.approval_id === ev.approval_id);
        if (a) {
          a.decision = ev.decision;
          a.decidedAt ??= Date.now();
        }
        break;
      }
      case "usage":
        chat.usage.input += ev.input_tokens;
        chat.usage.output += ev.output_tokens;
        if (ev.cost_usd != null) chat.usage.cost = (chat.usage.cost ?? 0) + ev.cost_usd;
        break;
      case "error":
        chat.items.push({ kind: "error", id: nid(), at: Date.now(), message: ev.message, retryable: ev.retryable });
        break;
      case "step_limit":
        chat.items.push({ kind: "limit", id: nid(), at: Date.now(), steps: ev.steps });
        break;
      case "done":
        // Remembered: `send` may get this run's id only after its end.
        this.over.add(runId);
        chat.runId = null;
        this.abortOpenTools(sessionId, chat);
        break;
    }
  }

  /** The run ended: tool calls still without a result never get one. */
  private abortOpenTools(sessionId: string, chat: Chat) {
    for (const i of chat.items)
      if (i.kind === "tool" && !i.done) {
        i.done = true;
        i.aborted = true;
        i.doneAt = Date.now();
        this.log(sessionId, chat, i);
      }
  }
}

/** The run_command call still waiting for its approval: the newest unfinished one running
 *  `command`, or failing that the newest unfinished run_command without an approval. */
export function matchCall(items: readonly ChatItem[], command: string): string | null {
  let fallback: string | null = null;
  for (let k = items.length - 1; k >= 0; k--) {
    const i = items[k];
    if (i.kind !== "tool" || i.name !== "run_command" || i.done) continue;
    if (items.some((a) => a.kind === "approval" && a.callId === i.callId)) continue;
    if (i.input.command === command) return i.callId;
    fallback ??= i.callId;
  }
  return fallback;
}

export const agent = new Agent();
