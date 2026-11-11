import { describe, expect, it } from "vitest";
import type { ApprovalItem, ChatItem, ToolItem } from "../stores/agent.svelte";
import type { CommandResult } from "../api/types";
import { agentStatus, buildActivity, buildSummary, filterActivity, latestText, runStart } from "./agentActivity";

let n = 0;
const user = (text: string): ChatItem => ({ kind: "user", id: `u${++n}`, at: n, text });
const said = (text: string): ChatItem => ({ kind: "assistant", id: `a${++n}`, at: n, text, thinking: "" });
const res = (command: string, extra: Partial<CommandResult> = {}): CommandResult => ({
  state: "completed",
  exit_code: 0,
  output: "ok",
  truncated: false,
  output_id: null,
  duration_ms: 12,
  command,
  note: null,
  ...extra,
});
const tool = (name: string, input: Record<string, unknown>, output: unknown = null, done = output != null): ToolItem => ({
  kind: "tool",
  id: `t${++n}`,
  at: n,
  callId: `c${n}`,
  name,
  input,
  output,
  isError: false,
  done,
  doneAt: done ? n + 1 : null,
  aborted: false,
  actor: "Claude · sonnet",
});
const approval = (call: ToolItem | null, command: string, decision: ApprovalItem["decision"] = null, risk: "read" | "write" | "destructive" = "read"): ApprovalItem => ({
  kind: "approval",
  id: `p${++n}`,
  at: n,
  req: { approval_id: `ap${n}`, session_id: "s", actor_label: "Claude", command, reason: null, risk, policy_note: null },
  decision,
  decidedAt: decision ? n : null,
  callId: call?.callId ?? null,
  expired: false,
});

describe("activity", () => {
  it("joins approvals to their calls and derives statuses", () => {
    const c1 = tool("run_command", { command: "df -h" }, res("df -h"));
    const c2 = tool("run_command", { command: "rm -rf x" }, res("rm -rf ./x"));
    const c3 = tool("run_command", { command: "uptime" }, res("uptime", { exit_code: 3 }));
    const c4 = tool("run_command", { command: "reboot" }, res("reboot", { state: "denied", exit_code: null }));
    const items: ChatItem[] = [
      user("check"),
      c1,
      approval(c1, "df -h", { type: "approve" }),
      said("text"),
      c2,
      approval(c2, "rm -rf x", { type: "approve_edited", command: "rm -rf ./x" }, "destructive"),
      c3,
      c4,
    ];
    const act = buildActivity(items);
    expect(act.map((e) => e.type)).toEqual(["prompt", "tool", "tool", "tool", "tool"]);
    const tools = act.filter((e) => e.type === "tool");
    expect(tools.map((e) => e.approvalStatus)).toEqual(["approved", "edited", "auto", "blocked"]);
    expect(tools.map((e) => e.exec)).toEqual(["completed", "completed", "failed", "denied"]);
    expect(tools[1].command).toBe("rm -rf ./x");
    expect(tools[1].risk).toBe("destructive");
    expect(tools[2].exitCode).toBe(3);
    expect(tools[0].durationMs).toBe(12);
  });

  it("pending approval: waiting; unmatched approvals stay visible", () => {
    const c = tool("run_command", { command: "ls" });
    const items = [user("x"), c, approval(c, "ls"), approval(null, "other")];
    const act = buildActivity(items);
    const t = act.find((e) => e.type === "tool")!;
    expect(t.type === "tool" && [t.approvalStatus, t.exec]).toEqual(["pending", "waiting"]);
    expect(act.at(-1)!.type).toBe("approval");
    expect(filterActivity(act, "pending", "all")).toHaveLength(2);
  });

  it("groups consecutive read_output / session_info calls", () => {
    const items = [
      tool("session_info", {}, { ok: 1 }),
      tool("read_output", { output_id: "1" }, "a"),
      tool("read_output", { output_id: "1" }, "b"),
      tool("read_output", { output_id: "1" }, "c"),
      tool("run_command", { command: "ls" }, res("ls")),
      tool("read_output", { output_id: "2" }, "d"),
    ];
    const act = buildActivity(items);
    expect(act.map((e) => (e.type === "group" ? `group:${e.tool}:${e.entries.length}` : e.type === "tool" ? e.tool : e.type))).toEqual([
      "session_info",
      "group:read_output:3",
      "run_command",
      "read_output",
    ]);
  });

  it("filters by status and tool type", () => {
    const items = [
      user("x"),
      tool("run_command", { command: "a" }, res("a", { exit_code: 1 })),
      tool("run_command", { command: "b" }, res("b")),
      tool("propose_file_edit", { path: "/etc/x" }, { outcome: "saved", path: "/etc/x" }),
      { kind: "error", id: "e1", at: 99, message: "boom", retryable: false } as ChatItem,
    ];
    const act = buildActivity(items);
    expect(filterActivity(act, "failed", "all").map((e) => e.type)).toEqual(["tool", "error"]);
    expect(filterActivity(act, "all", "file").map((e) => (e.type === "tool" ? e.tool : e.type))).toEqual(["propose_file_edit"]);
    expect(filterActivity(act, "all", "all")).toBe(act);
  });

  it("stays linear with many items", () => {
    const items: ChatItem[] = [];
    for (let i = 0; i < 2000; i++) items.push(tool("run_command", { command: `echo ${i}` }, res(`echo ${i}`)));
    const t0 = performance.now();
    const act = buildActivity(items);
    expect(act).toHaveLength(2000);
    expect(performance.now() - t0).toBeLessThan(200);
  });
});

describe("agent status", () => {
  it("covers each state", () => {
    expect(agentStatus([], false, false)).toBe("idle");
    expect(agentStatus([user("x")], true, false)).toBe("thinking");
    expect(agentStatus([user("x"), tool("run_command", { command: "ls" })], true, false)).toBe("running");
    expect(agentStatus([user("x"), tool("read_output", {})], true, false)).toBe("thinking");
    const c = tool("run_command", { command: "ls" });
    expect(agentStatus([user("x"), c, approval(c, "ls")], true, false)).toBe("waiting");
    expect(agentStatus([user("x"), said("done")], false, false)).toBe("completed");
    expect(agentStatus([user("x"), { kind: "error", id: "e", at: 1, message: "m", retryable: true }], false, false)).toBe("failed");
    expect(agentStatus([user("x"), said("a")], false, true)).toBe("cancelled");
    expect(agentStatus([user("x"), { kind: "limit", id: "l", at: 1, steps: 50 }], false, false)).toBe("limit");
    // An error in an earlier run does not mark the latest one failed.
    expect(agentStatus([user("x"), { kind: "error", id: "e", at: 1, message: "m", retryable: true }, user("y"), said("ok")], false, false)).toBe("completed");
  });
});

describe("summary", () => {
  it("uses real items only", () => {
    const c1 = tool("run_command", { command: "df -h" }, res("df -h"));
    const c2 = tool("run_command", { command: "systemctl restart nginx" }, res("systemctl restart nginx"));
    const c3 = tool("run_command", { command: "false" }, res("false", { exit_code: 1 }));
    const items: ChatItem[] = [
      user("old request"),
      said("old answer"),
      user("Free some disk"),
      c1,
      approval(c1, "df -h", { type: "approve" }),
      c2,
      approval(c2, "systemctl restart nginx", { type: "approve" }, "write"),
      c3,
      tool("propose_file_edit", { path: "/etc/nginx/nginx.conf" }, { outcome: "rejected", path: "/etc/nginx/nginx.conf" }),
      said("All done."),
      user("Continue."),
      said("Nothing else."),
    ];
    const from = runStart(items, ["Continue."]);
    expect(items[from]).toMatchObject({ text: "Free some disk" });
    const s = buildSummary(items, from);
    expect(s.objective).toBe("Free some disk");
    expect(s.commands.map((x) => x.command)).toEqual(["df -h", "systemctl restart nginx", "false"]);
    expect(s.errors.map((e) => e.entry?.command)).toEqual(["false"]);
    expect(s.changes.map((c) => [c.kind, c.text, c.detail, c.applied])).toEqual([
      ["command", "systemctl restart nginx", "write", true],
      ["file", "/etc/nginx/nginx.conf", "rejected", false],
    ]);
    expect(s.unclassified).toBe(1);
    expect(latestText(items, from)).toBe("Nothing else.");
    expect(buildSummary(items, 0).prompts).toBe(3);
    expect(buildSummary([], 0)).toMatchObject({ objective: null, commands: [], errors: [], changes: [] });
  });
});
