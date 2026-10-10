import { describe, expect, it } from "vitest";
import { createMockApi } from "./mock";
import { b64ToStr, strToB64 } from "../util/b64";
import { needsMremotengPassword } from "../util/importers";
import type { AgentEvent } from "./types";

const wait = (cond: () => boolean, ms = 5000) =>
  new Promise<void>((res, rej) => {
    const t0 = Date.now();
    const iv = setInterval(() => {
      if (cond()) (clearInterval(iv), res());
      else if (Date.now() - t0 > ms) (clearInterval(iv), rej(new Error("timeout")));
    }, 10);
  });

describe("mock api", () => {
  it("creates, locks and unlocks the vault", async () => {
    const api = createMockApi();
    expect(await api.vaultStatus()).toMatchObject({ initialized: false });
    const { secret_key } = await api.vaultCreate("correct horse battery");
    expect(secret_key).toMatch(/^A3-/);
    await api.vaultLock();
    expect((await api.vaultStatus()).locked).toBe(true);
    await expect(api.vaultUnlock("wrong")).rejects.toMatchObject({ kind: "auth" });
    await api.vaultUnlock("correct horse battery");
    expect((await api.vaultStatus()).locked).toBe(false);
  });

  it("seeds prod/staging/dev connections and saves new ones", async () => {
    const api = createMockApi();
    const list = await api.listConnections();
    const envs = new Set(list.map((c) => c.env));
    expect(envs.has("prod") && envs.has("staging") && envs.has("dev")).toBe(true);
    const c = { ...list[0], id: "x", name: "new" };
    await api.saveConnection(c);
    expect((await api.listConnections()).some((x) => x.id === "x")).toBe(true);
  });

  it("runs a fake shell that answers commands", async () => {
    const api = createMockApi();
    let out = "";
    await api.on("session://data", (e) => (out += b64ToStr(e.data_b64)));
    const dev = (await api.listConnections()).find((c) => c.name === "dev-box")!;
    const s = await api.openSession(dev.id, 80, 24);
    await wait(() => out.includes("$ "));
    await api.sessionWrite(s.session_id, strToB64("df -h\r"));
    await wait(() => out.includes("/var/lib/docker"));
    await api.sessionWrite(s.session_id, strToB64("cat /etc/os-release\r"));
    await wait(() => out.includes("Ubuntu 24.04"));
  });

  it("streams an agent run with an approval, then types into the terminal", async () => {
    const api = createMockApi();
    const events: AgentEvent[] = [];
    const marks: string[] = [];
    await api.on("agent://event", (e) => events.push(e.event));
    await api.on("session://agent_mark", (e) => marks.push(e.command));
    const dev = (await api.listConnections()).find((c) => c.name === "dev-box")!;
    const s = await api.openSession(dev.id, 80, 24);
    const [p] = await api.listProviders();
    await api.agentSend(s.session_id, { type: "built_in", provider: p.id, model: p.default_model }, "why is disk full?");
    await wait(() => events.some((e) => e.type === "approval_requested"));
    const req = events.find((e) => e.type === "approval_requested")!;
    expect(req.type === "approval_requested" && req.command).toBe("df -h");
    if (req.type === "approval_requested") await api.approvalDecide(req.approval_id, { type: "approve" });
    await wait(() => events.some((e) => e.type === "done"), 10000);
    expect(marks).toEqual(["df -h"]);
    const result = events.find((e) => e.type === "tool_result");
    expect(result && result.type === "tool_result" && (result.output as { exit_code: number }).exit_code).toBe(0);
    expect(events.some((e) => e.type === "usage")).toBe(true);
  }, 15000);

  it("detects harnesses and previews/applies imports by index", async () => {
    const api = createMockApi();
    const h = await api.detectHarnesses();
    expect(h.map((x) => x.kind)).toEqual(["claude_code", "codex", "open_code"]);
    const [src] = await api.importDetect();
    const preview = await api.importPreview({ source_id: src.id });
    expect(preview.duplicates[0].index).toBe(0);
    const before = (await api.listConnections()).length;
    const { imported } = await api.importApply(preview.preview_id, [0]);
    expect(imported).toBe(preview.connections.length - 1);
    expect((await api.listConnections()).length).toBe(before + imported);
    await expect(api.importApply(preview.preview_id, [])).rejects.toMatchObject({ kind: "not_found" });
  });

  it("previews a file import and asks for an mRemoteNG password", async () => {
    const api = createMockApi();
    const preview = await api.importPreview({ tool: "securecrt", file_path: "/tmp/export.xml" });
    expect(preview.connections.length).toBeGreaterThan(0);
    await expect(api.importPreview({ tool: "securecrt", file_path: "export.xml" })).rejects.toMatchObject({ kind: "io" });
    const locked = api.importPreview({ tool: "mremoteng", file_path: "/tmp/confCons.xml" });
    await expect(locked).rejects.toMatchObject({ kind: "auth" });
    expect(needsMremotengPassword("mremoteng", await locked.catch((e: unknown) => e))).toBe(true);
    const ok = await api.importPreview({ tool: "mremoteng", file_path: "/tmp/confCons.xml", mremoteng_password: "secret" });
    expect(ok.preview_id).toBeTruthy();
  });
});
