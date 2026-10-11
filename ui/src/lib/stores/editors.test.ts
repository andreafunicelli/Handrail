import { describe, expect, it, vi } from "vitest";
import { api } from "../api";
import { editors } from "./editors.svelte";
import type { FileBackend } from "../util/fileBackends";

const proposal = {
  proposal_id: "p1",
  session_id: "s1",
  path: "/etc/app.conf",
  root: false,
  original: "a = 1\n",
  stamp: { size: 6, modified: 1, hash: "h1" },
  proposed: "a = 2\n",
  reason: null,
  actor_label: "Agent",
};

describe("editor proposals", () => {
  it("answers the agent once: saved after a successful save", async () => {
    const decide = vi.spyOn(api, "editProposalDecide").mockResolvedValue();
    const id = editors.propose({ ...proposal, proposal_id: "ok" });
    const d = editors.docs[id];
    d.fs = { ...d.fs, writeText: async () => ({ size: 6, modified: 2, hash: "h2" }) } as FileBackend;
    await editors.accept(id);
    expect(decide).toHaveBeenCalledWith("ok", "saved", null);
    expect(editors.docs[id].proposal).toBeNull();
    expect(editors.docs[id].text).toBe("a = 2\n");
    editors.close(id);
    decide.mockRestore();
  });

  it("reloading after a failed accepted save tells the agent it was not saved", async () => {
    const decide = vi.spyOn(api, "editProposalDecide").mockResolvedValue();
    const id = editors.propose(proposal);
    const d = editors.docs[id];
    d.fs = {
      ...d.fs,
      writeText: async () => Promise.reject({ kind: "busy", message: "changed_on_server" }),
      readText: async () => ({ text: "a = 9\n", stamp: { size: 6, modified: 3, hash: "h9" } }),
    } as FileBackend;
    await editors.accept(id);
    expect(editors.docs[id].conflict).toBe(true);
    expect(decide).not.toHaveBeenCalled();
    await editors.load(id);
    expect(decide).toHaveBeenCalledWith("p1", "rejected", expect.stringContaining("reloaded"));
    expect(editors.docs[id].proposal).toBeNull();
    expect(editors.docs[id].text).toBe("a = 9\n");
    // A later save of unrelated edits must not report the old proposal as saved.
    d.fs = { ...d.fs, writeText: async () => ({ size: 6, modified: 4, hash: "h10" }) } as FileBackend;
    editors.docs[id].text = "a = 10\n";
    await editors.save(id);
    expect(decide).toHaveBeenCalledTimes(1);
    editors.close(id);
    decide.mockRestore();
  });
});
