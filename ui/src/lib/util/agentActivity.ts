/**
 * The Summary and Activity views of the AI sidebar, derived from the session's chat items only
 * (real events: tool calls, results, approvals, errors). Text deltas are never read, so the
 * derivations do not re-run per streamed token.
 */
import type { CommandResult, RiskClass } from "../api/types";
import type { ApprovalItem, ChatItem, ToolItem } from "../stores/agent.svelte";

export type ApprovalStatus = "pending" | "approved" | "edited" | "denied" | "auto" | "blocked" | "none";
export type ExecStatus = "running" | "waiting" | "completed" | "failed" | "timed_out" | "interrupted" | "needs_user" | "denied" | "rejected" | "error" | "no_result";
export type ToolKind = "command" | "file" | "db" | "other";

export interface ToolActivity {
  type: "tool";
  id: string;
  at: number;
  doneAt: number | null;
  tool: string;
  kind: ToolKind;
  item: ToolItem;
  /** The command that ran (the edited one when the user changed it), else the requested one. */
  command: string | null;
  /** File path of a file edit. */
  path: string | null;
  result: CommandResult | null;
  approval: ApprovalItem | null;
  approvalStatus: ApprovalStatus;
  exec: ExecStatus;
  risk: RiskClass | null;
  exitCode: number | null;
  durationMs: number | null;
  actor: string | null;
}

export type ActivityEntry =
  | ToolActivity
  | { type: "group"; id: string; at: number; tool: string; entries: ToolActivity[] }
  | { type: "approval"; id: string; at: number; item: ApprovalItem }
  | { type: "prompt"; id: string; at: number; text: string }
  | { type: "error"; id: string; at: number; message: string }
  | { type: "limit"; id: string; at: number; steps: number };

/** Tools that only look: consecutive calls are folded into one row. */
export const QUIET_TOOLS = new Set(["read_output", "session_info", "list_sessions", "list_connections"]);

export function toolKind(name: string): ToolKind {
  if (name === "run_command" || name === "send_keys") return "command";
  if (name === "propose_file_edit") return "file";
  if (name.startsWith("db_") || name === "list_databases") return "db";
  return "other";
}

export function commandResult(output: unknown): CommandResult | null {
  return output && typeof output === "object" && "state" in output ? (output as CommandResult) : null;
}

function approvalStatusOf(tool: ToolItem, a: ApprovalItem | null, result: CommandResult | null): ApprovalStatus {
  if (a) {
    if (!a.decision) return "pending";
    return a.decision.type === "approve" ? "approved" : a.decision.type === "approve_edited" ? "edited" : "denied";
  }
  if (tool.name !== "run_command") return "none";
  if (!tool.done) return "none"; // not known yet: the approval may still arrive
  return result?.state === "denied" ? "blocked" : "auto";
}

function execOf(tool: ToolItem, a: ApprovalItem | null, result: CommandResult | null): ExecStatus {
  if (!tool.done) return a && !a.decision ? "waiting" : "running";
  if (tool.aborted && tool.output == null) return "no_result";
  if (result) {
    if (result.state === "completed") return result.exit_code != null && result.exit_code !== 0 ? "failed" : "completed";
    return result.state;
  }
  const o = tool.output as { outcome?: unknown } | null;
  if (o && typeof o === "object" && typeof o.outcome === "string") {
    if (o.outcome === "rejected") return "rejected";
    if (o.outcome === "failed") return "error";
  }
  return tool.isError ? "error" : "completed";
}

export function isFailedExec(s: ExecStatus) {
  return s === "failed" || s === "error" || s === "timed_out" || s === "interrupted" || s === "no_result";
}

export function toolActivity(tool: ToolItem, approval: ApprovalItem | null): ToolActivity {
  const result = commandResult(tool.output);
  const str = (v: unknown) => (typeof v === "string" ? v : null);
  const edited = approval?.decision?.type === "approve_edited" ? approval.decision.command : null;
  const kind = toolKind(tool.name);
  return {
    type: "tool",
    id: tool.id,
    at: tool.at,
    doneAt: tool.doneAt,
    tool: tool.name,
    kind,
    item: tool,
    command: result?.command ?? edited ?? str(tool.input.command) ?? str(tool.input.keys) ?? str(tool.input.sql) ?? approval?.req.command ?? null,
    path: kind === "file" ? str(tool.input.path) : null,
    result,
    approval,
    approvalStatus: approvalStatusOf(tool, approval, result),
    exec: execOf(tool, approval, result),
    risk: approval?.req.risk ?? null,
    exitCode: result?.exit_code ?? null,
    durationMs: result ? result.duration_ms : tool.doneAt != null ? tool.doneAt - tool.at : null,
    actor: approval?.req.actor_label ?? tool.actor,
  };
}

/** Each tool call with its approval, plus approvals that matched no call; in order. */
export function toolActivities(items: readonly ChatItem[]): { tools: ToolActivity[]; orphans: ApprovalItem[] } {
  const byCall = new Map<string, ApprovalItem>();
  const approvals: ApprovalItem[] = [];
  for (const i of items)
    if (i.kind === "approval") {
      approvals.push(i);
      if (i.callId) byCall.set(i.callId, i);
    }
  const tools: ToolActivity[] = [];
  for (const i of items) if (i.kind === "tool") tools.push(toolActivity(i, byCall.get(i.callId) ?? null));
  const linked = new Set(tools.map((x) => x.approval?.id));
  return { tools, orphans: approvals.filter((a) => !linked.has(a.id)) };
}

/** The timeline, oldest first: prompts, tool calls (quiet ones grouped), errors, limits. */
export function buildActivity(items: readonly ChatItem[]): ActivityEntry[] {
  const { tools, orphans } = toolActivities(items);
  const toolById = new Map(tools.map((x) => [x.id, x]));
  const orphanIds = new Set(orphans.map((a) => a.id));
  const out: ActivityEntry[] = [];
  const push = (e: ActivityEntry) => {
    const prev = out.at(-1);
    if (e.type === "tool" && QUIET_TOOLS.has(e.tool) && prev) {
      if (prev.type === "group" && prev.tool === e.tool) return void prev.entries.push(e);
      if (prev.type === "tool" && prev.tool === e.tool) {
        out[out.length - 1] = { type: "group", id: `g-${prev.id}`, at: prev.at, tool: e.tool, entries: [prev, e] };
        return;
      }
    }
    out.push(e);
  };
  for (const i of items) {
    if (i.kind === "tool") push(toolById.get(i.id)!);
    else if (i.kind === "approval") {
      if (orphanIds.has(i.id)) push({ type: "approval", id: i.id, at: i.at, item: i });
    } else if (i.kind === "user") push({ type: "prompt", id: i.id, at: i.at, text: i.text });
    else if (i.kind === "error") push({ type: "error", id: i.id, at: i.at, message: i.message });
    else if (i.kind === "limit") push({ type: "limit", id: i.id, at: i.at, steps: i.steps });
  }
  return out;
}

export type ActivityFilter = "all" | "failed" | "pending";

function toolMatches(e: ToolActivity, status: ActivityFilter, kind: ToolKind | "all") {
  if (kind !== "all" && e.kind !== kind) return false;
  if (status === "failed") return isFailedExec(e.exec);
  if (status === "pending") return e.approvalStatus === "pending" || e.exec === "waiting" || e.exec === "running";
  return true;
}

/** Apply the status and tool-type filters (prompts and limits show only when unfiltered). */
export function filterActivity(entries: readonly ActivityEntry[], status: ActivityFilter, kind: ToolKind | "all"): ActivityEntry[] {
  if (status === "all" && kind === "all") return entries as ActivityEntry[];
  const out: ActivityEntry[] = [];
  for (const e of entries) {
    if (e.type === "tool") {
      if (toolMatches(e, status, kind)) out.push(e);
    } else if (e.type === "group") {
      const kept = e.entries.filter((x) => toolMatches(x, status, kind));
      if (kept.length) out.push(kept.length === e.entries.length ? e : { ...e, entries: kept });
    } else if (e.type === "approval") {
      // An approval that matched no call (e.g. from a harness) is a command request.
      const typeOk = kind === "all" || kind === "command";
      const statusOk = status === "pending" ? !e.item.decision : status === "all";
      if (typeOk && statusOk) out.push(e);
    } else if (e.type === "error") {
      if (status === "failed" && kind === "all") out.push(e);
    }
  }
  return out;
}

export type AgentStatus = "idle" | "thinking" | "running" | "waiting" | "completed" | "failed" | "cancelled" | "limit";

/** Where the agent is, from the chat state (not the terminal's connection state). */
export function agentStatus(items: readonly ChatItem[], running: boolean, cancelled: boolean): AgentStatus {
  if (items.some((i) => i.kind === "approval" && !i.decision)) return "waiting";
  if (running) {
    for (let k = items.length - 1; k >= 0; k--) {
      const i = items[k];
      if (i.kind === "user") break;
      if (i.kind === "tool" && !i.done) return toolKind(i.name) === "command" ? "running" : "thinking";
    }
    return "thinking";
  }
  if (!items.length) return "idle";
  if (cancelled) return "cancelled";
  for (let k = items.length - 1; k >= 0; k--) {
    const i = items[k];
    if (i.kind === "user") break;
    if (i.kind === "error") return "failed";
    if (i.kind === "limit") return "limit";
  }
  return items.at(-1)?.kind === "user" ? "idle" : "completed";
}

export interface Change {
  kind: "file" | "command";
  id: string;
  at: number;
  /** Path of the file, or the command. */
  text: string;
  /** File edit: saved / rejected / failed / pending; command: its risk. */
  detail: string;
  applied: boolean;
}

export interface RunSummary {
  /** The request that started the scope (the user's own words). */
  objective: string | null;
  startedAt: number | null;
  prompts: number;
  commands: ToolActivity[];
  tools: number;
  errors: { id: string; at: number; message: string | null; entry: ToolActivity | null }[];
  changes: Change[];
  /** Commands that ran without an approval (their risk is not known here). */
  unclassified: number;
  pending: number;
}

/** Index of the item that starts the latest run: the last user message that is not one of
 *  `followUps` (such as "Continue."). */
export function runStart(items: readonly ChatItem[], followUps: readonly string[] = []): number {
  let last = -1;
  for (let k = items.length - 1; k >= 0; k--) {
    const i = items[k];
    if (i.kind !== "user") continue;
    if (last === -1) last = k;
    if (!followUps.includes(i.text.trim())) return k;
  }
  return last === -1 ? 0 : last;
}

export function buildSummary(items: readonly ChatItem[], from = 0): RunSummary {
  const scope = items.slice(from);
  const { tools } = toolActivities(scope);
  const first = scope.find((i) => i.kind === "user");
  const commands = tools.filter((x) => x.kind === "command");
  const errors: RunSummary["errors"] = [];
  const changes: Change[] = [];
  let unclassified = 0;
  for (const i of scope) if (i.kind === "error") errors.push({ id: i.id, at: i.at, message: i.message, entry: null });
  for (const x of tools) {
    if (isFailedExec(x.exec)) errors.push({ id: x.id, at: x.at, message: x.result?.note ?? null, entry: x });
    if (x.kind === "file" && x.path) {
      const o = (x.item.output as { outcome?: string } | null)?.outcome;
      changes.push({ kind: "file", id: x.id, at: x.at, text: x.path, detail: o ?? (x.item.done ? "unknown" : "pending"), applied: o === "saved" });
    } else if (x.tool === "run_command" && x.exec !== "denied" && x.exec !== "waiting" && x.exec !== "running") {
      if (x.risk === "write" || x.risk === "destructive") changes.push({ kind: "command", id: x.id, at: x.at, text: x.command ?? "", detail: x.risk, applied: x.result?.state === "completed" });
      else if (x.approvalStatus === "auto") unclassified++;
    }
  }
  errors.sort((a, b) => a.at - b.at);
  return {
    objective: first?.kind === "user" ? first.text : null,
    startedAt: first?.at ?? scope[0]?.at ?? null,
    prompts: scope.filter((i) => i.kind === "user").length,
    commands,
    tools: tools.length,
    errors,
    changes,
    unclassified,
    pending: scope.filter((i) => i.kind === "approval" && !i.decision).length,
  };
}

/** The agent's latest text from `from` on, verbatim (reads only the last assistant item). */
export function latestText(items: readonly ChatItem[], from = 0): string | null {
  for (let k = items.length - 1; k >= from; k--) {
    const i = items[k];
    if (i.kind === "assistant" && i.text.trim()) return i.text;
  }
  return null;
}
