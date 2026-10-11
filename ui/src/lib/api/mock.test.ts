import { describe, expect, it } from "vitest";
import { createMockApi } from "./mock";
import { b64ToStr, strToB64 } from "../util/b64";
import { needsMremotengPassword } from "../util/importers";
import type { AgentEvent, SyncStatus } from "./types";

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

  it("asks for the password of a connection without saved credentials", async () => {
    const api = createMockApi();
    const pi = (await api.listConnections()).find((c) => c.auth.type === "prompt")!;
    await expect(api.openSession(pi.id, 80, 24)).rejects.toMatchObject({ kind: "auth" });
    const asked: string[] = [];
    await api.on("session://auth_prompt", (e) => {
      asked.push(e.instructions);
      void api.answerAuthPrompt(e.request_id, [asked.length === 1 ? "" : "secret"]);
    });
    const s = await api.openSession(pi.id, 80, 24, "ref-1");
    expect(s.connection_id).toBe(pi.id);
    expect(asked).toEqual(["", "Permission denied, please try again."]);
  });

  it("saves, lists and deletes snippets", async () => {
    const api = createMockApi();
    const before = await api.listSnippets();
    const s = await api.saveSnippet({ id: "s-1", name: "Uptime", command: "uptime", description: "", tags: [] });
    expect((await api.listSnippets()).length).toBe(before.length + 1);
    await api.saveSnippet({ ...s, command: "uptime -p" });
    expect((await api.listSnippets()).find((x) => x.id === "s-1")?.command).toBe("uptime -p");
    await expect(api.saveSnippet({ ...s, name: " " })).rejects.toMatchObject({ kind: "invalid" });
    await api.deleteSnippet("s-1");
    expect(await api.listSnippets()).toEqual(before);
  });

  it("browses, transfers and edits files over SFTP", async () => {
    const api = createMockApi();
    const conn = (await api.listConnections()).find((c) => c.auth.type !== "prompt")!;
    const s = await api.openSession(conn.id, 80, 24);
    const home = await api.sftpHome(s.session_id);
    expect((await api.sftpList(s.session_id, home)).map((e) => e.name)).toContain("logs");
    const seen: number[] = [];
    await api.on("transfer://progress", (p) => seen.push(p.done));
    const remote = await api.sftpUpload(s.session_id, "/tmp/x.bin", home, "t1");
    expect(remote).toBe(`${home}/x.bin`);
    expect(seen.at(-1)).toBeGreaterThan(0);
    await api.sftpMkdir(s.session_id, `${home}/dir`);
    await api.sftpRename(s.session_id, remote, `${home}/dir/y.bin`);
    await expect(api.sftpRemove(s.session_id, `${home}/dir`, true)).rejects.toBeTruthy();
    await api.sftpRemove(s.session_id, `${home}/dir/y.bin`, false);
    await api.sftpRemove(s.session_id, `${home}/dir`, true);
    await expect(api.sftpList(s.session_id, `${home}/dir`)).rejects.toMatchObject({ kind: "not_found" });
    const local = await api.openLocalSession(null, 80, 24);
    await expect(api.sftpHome(local.session_id)).rejects.toMatchObject({ kind: "invalid" });
  });

  it("keeps S3 accounts and browses buckets like the backend", async () => {
    const api = createMockApi();
    const [r2] = await api.listS3Accounts();
    expect(await api.s3Home(r2.id)).toBe("/backups");
    const aws = (await api.listS3Accounts()).find((a) => !a.bucket)!;
    expect(await api.s3Home(aws.id)).toBe("/");
    expect((await api.s3List(aws.id, "/")).map((e) => e.name)).toContain("backups");
    const top = await api.s3List(r2.id, "/backups");
    expect(top.map((e) => `${e.kind}:${e.name}`)).toEqual(["dir:db", "dir:site", "file:README.txt"]);
    const up = await api.s3Upload(r2.id, "/tmp/new.txt", "/backups/db", "t1");
    expect(up).toBe("/backups/db/new.txt");
    await api.s3Rename(r2.id, up, "/backups/db/renamed.txt", false);
    await expect(api.s3Rename(r2.id, "/backups/db", "/backups/x", true)).rejects.toMatchObject({ kind: "invalid" });
    await expect(api.s3Remove(r2.id, "/backups/db", true)).rejects.toMatchObject({ kind: "invalid" });
    await api.s3Mkdir(r2.id, "/backups/empty");
    await api.s3Remove(r2.id, "/backups/empty", true);
    await expect(api.s3Mkdir(r2.id, "/newbucket")).rejects.toMatchObject({ kind: "invalid" });
    await expect(api.saveS3Account({ ...r2, id: "x", secret: null }, null)).rejects.toMatchObject({ kind: "invalid" });
    await api.deleteS3Account(r2.id);
    expect((await api.listS3Accounts()).some((a) => a.id === r2.id)).toBe(false);
  });

  it("opens only https links", async () => {
    const api = createMockApi();
    await expect(api.openUrl("javascript:alert(1)")).rejects.toThrow();
    await expect(api.openUrl("http://github.com/login/device")).rejects.toThrow();
  });

  it("reports the version and installs an update with progress", async () => {
    const api = createMockApi();
    expect(await api.appVersion()).toMatch(/^\d+\.\d+\.\d+$/);
    expect(await api.checkForUpdate()).toBeNull();
    const seen: (number | null)[] = [];
    await api.installUpdate((p) => seen.push(p));
    expect(seen[0]).toBe(0);
    expect(seen[seen.length - 1]).toBe(1);
  });

  it("syncs with status events, auto-sync and disconnect", async () => {
    const api = createMockApi();
    const seen: SyncStatus[] = [];
    await api.on("sync://status", (s) => seen.push(s));
    expect((await api.syncStatus()).auto).toBe(true);
    await expect(api.syncNow()).rejects.toMatchObject({ kind: "invalid" });
    const [p] = await api.syncFolderPresets();
    await api.syncConfigureFolder(p.path);
    const report = await api.syncNow();
    expect(report.pushed).toBeGreaterThan(0);
    expect(seen.map((s) => s.syncing)).toEqual([false, true, false]);
    const st = await api.syncStatus();
    expect(st.config).toEqual({ kind: "folder", path: p.path });
    expect(st.last_sync_ms).toBeTruthy();
    await api.syncSetAuto(false);
    expect((await api.syncStatus()).auto).toBe(false);
    await api.syncDisconnect();
    const off = await api.syncStatus();
    expect(off.config).toBeNull();
    expect(off.auto).toBe(false);
  });

  it("lists, takes and restores snapshots; restores and clears conflicts", async () => {
    const api = createMockApi();
    const before = await api.vaultSnapshots();
    expect(before.length).toBeGreaterThan(0);
    expect(before[0].created_ms).toBeGreaterThanOrEqual(before[before.length - 1].created_ms);
    const snap = await api.vaultSnapshotCreate();
    expect((await api.vaultSnapshots())[0].name).toBe(snap.name);
    await api.vaultSnapshotRestore(before[0].name);
    await expect(api.vaultSnapshotRestore("nope.snap")).rejects.toMatchObject({ kind: "not_found" });
    const [c] = await api.vaultConflicts();
    await api.vaultConflictRestore(c.file_id, c.recorded_at_ms);
    await expect(api.vaultConflictRestore(c.file_id, c.recorded_at_ms + 1)).rejects.toMatchObject({ kind: "not_found" });
    await api.vaultConflictsClear();
    expect(await api.vaultConflicts()).toEqual([]);
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
