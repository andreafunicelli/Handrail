import { describe, expect, it, vi } from "vitest";
import { findLeafBySession, leaf, leaves, mapLeaves, removeLeaf, setRatio, splitLeaf, type LayoutNode } from "./layout";

vi.mock("../terminal/host", () => ({
  terminals: { get: () => ({ write() {}, mark() {} }), dispose() {}, peek() {}, configure() {} },
}));

describe("layout tree", () => {
  it("splits a leaf right and down", () => {
    const a = leaf("s1", "A");
    const b = leaf("s2", "B");
    const c = leaf("s3", "C");
    let t: LayoutNode = splitLeaf(a, "A", b, "row");
    t = splitLeaf(t, "B", c, "col");
    expect(leaves(t).map((l) => l.sessionId)).toEqual(["s1", "s2", "s3"]);
    expect(t.type === "split" && t.dir).toBe("row");
    expect(t.type === "split" && t.b.type === "split" && t.b.dir).toBe("col");
  });
  it("inserts before when asked", () => {
    const t = splitLeaf(leaf("s1", "A"), "A", leaf("s2", "B"), "row", true);
    expect(leaves(t).map((l) => l.id)).toEqual(["B", "A"]);
  });
  it("removes leaves and collapses parents", () => {
    let t: LayoutNode = splitLeaf(leaf("s1", "A"), "A", leaf("s2", "B"), "row");
    t = splitLeaf(t, "B", leaf("s3", "C"), "col");
    const r1 = removeLeaf(t, "B")!;
    expect(leaves(r1).map((l) => l.id)).toEqual(["A", "C"]);
    expect(r1.type === "split" && r1.b.type).toBe("leaf");
    const r2 = removeLeaf(r1, "A")!;
    expect(r2).toEqual(leaf("s3", "C"));
    expect(removeLeaf(r2, "C")).toBeNull();
    expect(removeLeaf(r2, "nope")).toBe(r2);
  });
  it("clamps ratios", () => {
    const t = splitLeaf(leaf("s1", "A"), "A", leaf("s2", "B"), "row");
    const id = t.type === "split" ? t.id : "";
    const r = setRatio(t, id, 1.5);
    expect(r.type === "split" && r.ratio).toBe(0.9);
    expect((setRatio(t, id, -1) as { ratio: number }).ratio).toBe(0.1);
  });
  it("maps and finds leaves", () => {
    const t = splitLeaf(leaf("s1", "A"), "A", leaf("s2", "B"), "row");
    const m = mapLeaves(t, (l) => ({ ...l, sessionId: l.sessionId + "x" }));
    expect(findLeafBySession(m, "s2x")?.id).toBe("B");
    expect(findLeafBySession(t, "s2x")).toBeNull();
  });
});

describe("sessions store (tabs & splits)", () => {
  it("opens tabs, splits, merges tabs and closes panes", async () => {
    const { sessions } = await import("../stores/sessions.svelte");
    const { connections } = await import("../stores/connections.svelte");
    await connections.load();
    const [c1, c2] = connections.list;
    await sessions.open(c1.id);
    const tab1 = sessions.activeTabId!;
    await sessions.open(c2.id, "row");
    expect(sessions.tabs).toHaveLength(1);
    expect(leaves(sessions.activeTab!.layout)).toHaveLength(2);

    await sessions.openLocal();
    const tab2 = sessions.activeTabId!;
    expect(sessions.tabs).toHaveLength(2);

    // drag tab2 onto tab1's first pane → split down, tab2 disappears
    const firstLeaf = leaves(sessions.tabs.find((t) => t.id === tab1)!.layout)[0];
    sessions.mergeTab(tab2, tab1, firstLeaf.id, "col");
    expect(sessions.tabs).toHaveLength(1);
    expect(leaves(sessions.activeTab!.layout)).toHaveLength(3);

    // detach, then close
    const l = leaves(sessions.activeTab!.layout)[1];
    sessions.detachPane(tab1, l.id);
    expect(sessions.tabs).toHaveLength(2);
    sessions.closeTab(sessions.activeTabId!);
    expect(sessions.tabs).toHaveLength(1);
    for (const leafNode of leaves(sessions.activeTab!.layout)) sessions.closePane(sessions.activeTabId!, leafNode.id);
    expect(sessions.tabs).toHaveLength(0);
    expect(sessions.activeTabId).toBeNull();
  });

  it("saves workspaces with connection references", async () => {
    const { sessions } = await import("../stores/sessions.svelte");
    const { connections } = await import("../stores/connections.svelte");
    await sessions.open(connections.list[0].id);
    await sessions.openLocal(null, "row");
    const ws = sessions.saveWorkspace("ops");
    expect(leaves(ws.tabs[0]).map((l) => l.sessionId)).toEqual([connections.list[0].id, "local"]);
  });
});
