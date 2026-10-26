import { describe, expect, it } from "vitest";
import { Sessions, isRdp, rdpConnectionOf } from "./sessions.svelte";
import { connections } from "./connections.svelte";
import { leaf, splitLeaf } from "../util/layout";
import type { Connection, SessionInfo, SessionState } from "../api/types";

const info = (session_id: string, state: SessionState = "idle"): SessionInfo =>
  ({ session_id, connection_id: "c", title: session_id, env: "none", state, approval_mode: "confirm_all" }) as SessionInfo;

/** One tab: terminals a, b, c, a file browser of a, and a disconnected terminal d. */
function setup() {
  const s = new Sessions();
  const a = leaf("a");
  let layout = splitLeaf(a, a.id, leaf("b"), "row");
  layout = splitLeaf(layout, a.id, leaf("c"), "col");
  layout = splitLeaf(layout, a.id, leaf("files:a"), "col");
  layout = splitLeaf(layout, a.id, leaf("d"), "row");
  s.tabs = [{ id: "t", layout, activeLeaf: a.id }];
  for (const [id, st] of [["a"], ["b"], ["c", "agent_running"], ["d", "disconnected"]] as [string, SessionState?][]) s.infos[id] = info(id, st);
  return s;
}

describe("typing in all panes", () => {
  it("off: typing goes to its own terminal", () => {
    const s = setup();
    expect(s.inputTargets("a")).toEqual(["a"]);
    expect(s.broadcasting("t")).toBe(false);
  });

  it("on: live terminals only (no file browser, agent-driven or disconnected pane)", () => {
    const s = setup();
    s.toggleBroadcast("t");
    expect(s.broadcasting("t")).toBe(true);
    expect(s.inputTargets("a").sort()).toEqual(["a", "b"]);
    // Typing in a pane left out (the agent's) still reaches that pane itself.
    expect(s.inputTargets("c").sort()).toEqual(["a", "b", "c"]);
    s.toggleBroadcast("t");
    expect(s.inputTargets("b")).toEqual(["b"]);
  });

  it("one live terminal left: nothing to mirror", () => {
    const s = setup();
    s.toggleBroadcast("t");
    s.infos.b = info("b", "disconnected");
    expect(s.broadcasting("t")).toBe(false);
    expect(s.inputTargets("a")).toEqual(["a"]);
  });
});

describe("retrying a failed login", () => {
  it("opens a remote desktop once the connection has been made one", () => {
    const s = new Sessions();
    const l = leaf("pending:r");
    s.tabs = [{ id: "t", layout: l, activeLeaf: l.id }];
    s.pending.r = { ref: "r", connectionId: "w", label: "w", target: "w", prompt: null, error: "reset" };
    connections.list = [{ id: "w", kind: "rdp" } as Connection];
    s.retry("r");
    const id = (s.tabs[0].layout as { sessionId: string }).sessionId;
    expect(isRdp(id) && rdpConnectionOf(id)).toBe("w");
    expect(s.pending.r).toBeUndefined();
  });
});
