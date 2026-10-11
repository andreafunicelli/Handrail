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

export type ChatItem =
  | { kind: "user"; id: string; text: string }
  | { kind: "assistant"; id: string; text: string; thinking: string }
  | {
      kind: "tool";
      id: string;
      callId: string;
      name: string;
      input: Record<string, unknown>;
      output: unknown;
      isError: boolean;
      done: boolean;
    }
  | { kind: "approval"; id: string; req: ApprovalRequest; decision: ApprovalDecision | null }
  | { kind: "error"; id: string; message: string; retryable: boolean };

export interface Chat {
  items: ChatItem[];
  runId: string | null;
  usage: { input: number; output: number; cost: number | null };
  lastMessage: string | null;
}

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

  private started = false;

  async init() {
    if (this.started) return;
    this.started = true;
    await api.on("agent://event", (e) => this.apply(e.session_id, e.run_id, e.event));
    await api.on("approval://requested", (req) => this.addApproval(req));
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
      this.chats[sessionId] = { items: [], runId: null, usage: { input: 0, output: 0, cost: null }, lastMessage: null };
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
    return (this.chats[sessionId]?.items ?? []).filter(
      (i): i is Extract<ChatItem, { kind: "approval" }> => i.kind === "approval" && !i.decision,
    );
  }

  async send(sessionId: string, message: string) {
    const brain = this.brainFor(sessionId);
    if (!brain) return;
    const chat = this.chat(sessionId);
    chat.items.push({ kind: "user", id: nid(), text: message });
    chat.lastMessage = message;
    try {
      const { run_id } = await api.agentSend(sessionId, $state.snapshot(brain.brain) as Brain, message);
      chat.runId = run_id;
    } catch (e) {
      chat.items.push({ kind: "error", id: nid(), message: errorMessage(e), retryable: true });
    }
  }

  async cancel(sessionId: string) {
    const chat = this.chats[sessionId];
    if (!chat?.runId) return;
    const id = chat.runId;
    chat.runId = null;
    await api.agentCancel(id).catch(() => {});
  }

  async decide(sessionId: string, approvalId: string, decision: ApprovalDecision) {
    const item = this.chats[sessionId]?.items.find((i) => i.kind === "approval" && i.req.approval_id === approvalId);
    if (item && item.kind === "approval") item.decision = decision;
    try {
      await api.approvalDecide(approvalId, decision);
    } catch (e) {
      if (item && item.kind === "approval") item.decision = null;
      toasts.error(e);
    }
  }

  /** A reconnect replaced session `from` with `to`: the conversation and brain choice follow. */
  moveSession(from: string, to: string) {
    const chat = this.chats[from];
    if (chat) {
      this.chats[to] = { ...chat, runId: null };
      delete this.chats[from];
    }
    if (this.brainBySession[from]) {
      this.brainBySession[to] = this.brainBySession[from];
      delete this.brainBySession[from];
    }
  }

  clear(sessionId: string) {
    if (this.chats[sessionId]?.runId) return;
    delete this.chats[sessionId];
  }

  private addApproval(req: ApprovalRequest) {
    const chat = this.chat(req.session_id);
    if (chat.items.some((i) => i.kind === "approval" && i.req.approval_id === req.approval_id)) return;
    chat.items.push({ kind: "approval", id: nid(), req, decision: null });
  }

  /** Fold one normalized AgentEvent into the session's transcript. */
  apply(sessionId: string, runId: string, ev: AgentEvent) {
    const chat = this.chat(sessionId);
    if (ev.type !== "done" && !chat.runId) chat.runId = runId;
    const last = chat.items.at(-1);
    switch (ev.type) {
      case "text_delta":
      case "thinking_delta": {
        let a = last?.kind === "assistant" ? last : null;
        if (!a) {
          chat.items.push({ kind: "assistant", id: nid(), text: "", thinking: "" });
          a = chat.items.at(-1) as Extract<ChatItem, { kind: "assistant" }>;
        }
        if (ev.type === "text_delta") a.text += ev.text;
        else a.thinking += ev.text;
        break;
      }
      case "tool_call":
        chat.items.push({
          kind: "tool",
          id: nid(),
          callId: ev.call_id,
          name: ev.name,
          input: (ev.input && typeof ev.input === "object" ? ev.input : {}) as Record<string, unknown>,
          output: null,
          isError: false,
          done: false,
        });
        break;
      case "tool_result": {
        const t = chat.items.find((i) => i.kind === "tool" && i.callId === ev.call_id);
        if (t && t.kind === "tool") {
          t.output = ev.output;
          t.isError = ev.is_error;
          t.done = true;
        }
        break;
      }
      case "approval_requested": {
        const { type: _type, ...req } = ev;
        this.addApproval(req);
        break;
      }
      case "approval_resolved": {
        const a = chat.items.find((i) => i.kind === "approval" && i.req.approval_id === ev.approval_id);
        if (a && a.kind === "approval") a.decision = ev.decision;
        break;
      }
      case "usage":
        chat.usage.input += ev.input_tokens;
        chat.usage.output += ev.output_tokens;
        if (ev.cost_usd != null) chat.usage.cost = (chat.usage.cost ?? 0) + ev.cost_usd;
        break;
      case "error":
        chat.items.push({ kind: "error", id: nid(), message: ev.message, retryable: ev.retryable });
        break;
      case "done":
        if (chat.runId === runId || chat.runId === null) chat.runId = null;
        for (const i of chat.items) if (i.kind === "tool" && !i.done) i.done = true;
        break;
    }
  }
}

export const agent = new Agent();
