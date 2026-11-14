import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";
import type { ApprovalRequest } from "../api/types";
import { agent, chatBusy, type BrainOption } from "./agent.svelte";

const brain = { key: "k", brain: { type: "provider", provider_id: "p", model: "m" }, label: "Test", detail: "m", harness: null } as unknown as BrainOption;

/** agentSend that resolves only when the test says so, with `run_id`. */
function heldSend() {
  let release!: (run_id: string) => void;
  const send = vi.spyOn(api, "agentSend").mockImplementation(() => new Promise((res) => (release = (run_id) => res({ run_id }))));
  return { send, release: (id: string) => release(id) };
}

const flush = () => new Promise((r) => setTimeout(r, 0));

describe("agent run state", () => {
  beforeEach(() => {
    vi.spyOn(agent, "brainFor").mockReturnValue(brain);
  });
  afterEach(() => vi.restoreAllMocks());

  it("a second send while the first is starting starts nothing", async () => {
    const { send, release } = heldSend();
    const first = agent.send("s1", "hello");
    expect(chatBusy(agent.chats.s1)).toBe(true);
    await agent.send("s1", "hello again");
    expect(send).toHaveBeenCalledTimes(1);
    release("r1");
    await first;
    expect(agent.chats.s1.runId).toBe("r1");
    expect(agent.chats.s1.items.filter((i) => i.kind === "user")).toHaveLength(1);
  });

  it("stopping while starting stops the run once it has an id", async () => {
    const { release } = heldSend();
    const cancel = vi.spyOn(api, "agentCancel").mockResolvedValue(undefined);
    const sending = agent.send("s2", "go");
    await agent.cancel("s2");
    release("r2");
    await sending;
    expect(cancel).toHaveBeenCalledWith("r2");
    expect(agent.chats.s2.runId).toBeNull();
    // Its late output is not shown.
    agent.apply("s2", "r2", { type: "text_delta", text: "late" });
    expect(agent.chats.s2.items.some((i) => i.kind === "assistant")).toBe(false);
    expect(agent.chats.s2.runId).toBeNull();
  });

  it("a run that ends before its id arrives does not stay running", async () => {
    const { release } = heldSend();
    const sending = agent.send("s3", "quick");
    agent.apply("s3", "r3", { type: "text_delta", text: "done already" });
    agent.apply("s3", "r3", { type: "done" });
    release("r3");
    await sending;
    expect(agent.chats.s3.runId).toBeNull();
    expect(chatBusy(agent.chats.s3)).toBe(false);
  });

  it("events of an older run do not land in a newer one", async () => {
    vi.spyOn(api, "agentCancel").mockResolvedValue(undefined);
    vi.spyOn(api, "agentSend").mockResolvedValueOnce({ run_id: "old" }).mockResolvedValueOnce({ run_id: "new" });
    await agent.send("s4", "one");
    await agent.cancel("s4");
    await agent.send("s4", "two");
    agent.apply("s4", "old", { type: "text_delta", text: "stale" });
    agent.apply("s4", "old", { type: "done" });
    expect(agent.chats.s4.runId).toBe("new");
    expect(agent.chats.s4.items.some((i) => i.kind === "assistant")).toBe(false);
  });

  it("closing the session stops its run, denies waiting approvals and drops the chat", async () => {
    vi.spyOn(api, "agentSend").mockResolvedValue({ run_id: "r5" });
    const cancel = vi.spyOn(api, "agentCancel").mockResolvedValue(undefined);
    const decide = vi.spyOn(api, "approvalDecide").mockResolvedValue(undefined);
    await agent.send("s5", "work");
    const req: ApprovalRequest = { approval_id: "a5", session_id: "s5", actor_label: "Test", command: "rm x", reason: null, risk: "write", policy_note: null };
    agent.apply("s5", "r5", { type: "approval_requested", ...req });
    expect(agent.pendingApprovals("s5")).toHaveLength(1);
    agent.release("s5");
    await flush();
    expect(cancel).toHaveBeenCalledWith("r5");
    expect(decide).toHaveBeenCalledWith("a5", { type: "deny", message: null });
    expect(agent.chats.s5).toBeUndefined();
    // A late event does not bring it back.
    agent.apply("s5", "r5", { type: "text_delta", text: "late" });
    expect(agent.chats.s5).toBeUndefined();
  });
});
