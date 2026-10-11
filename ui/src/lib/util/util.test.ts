import { describe, expect, it } from "vitest";
import { fuzzyFilter, fuzzyScore } from "./fuzzy";
import { parseQuickConnect } from "./quickconnect";
import { escapeHtml, renderMarkdown } from "./markdown";
import { canSetApprovalMode, needsTypedConfirm } from "./approval";
import { buildTree, groupPath, isDescendant } from "./tree";
import { passwordStrength } from "../stores/vault.svelte";
import { joinArgs, splitArgs } from "./args";
import { formatBytes, relativeTime } from "./time";
import { syncChangedVault } from "../stores/vaultData";
import type { SyncStatus } from "../api/types";
import { joinPath, modeString, parentDir, validName } from "./files";
import { SUGGESTED_SNIPPETS, fillPlaceholders, placeholders, snippetInput } from "./snippets";
import { presetOf, s3Where } from "./s3";
import { reconnectDelay, retryable, shouldAutoReconnect } from "./reconnect";
import type { SessionInfo } from "../api/types";

describe("fuzzy", () => {
  it("matches subsequences and rejects non-matches", () => {
    expect(fuzzyScore("wb1", "web-01")).not.toBeNull();
    expect(fuzzyScore("xyz", "web-01")).toBeNull();
    expect(fuzzyScore("", "anything")).toBe(0);
  });
  it("ranks prefix and consecutive matches higher", () => {
    const a = fuzzyScore("web", "web-01")!;
    const b = fuzzyScore("web", "w-e-b-staging")!;
    expect(a).toBeGreaterThan(b);
  });
  it("filters and sorts across fields", () => {
    const items = [
      { name: "api-staging", host: "api.staging.acme.io" },
      { name: "web-01", host: "10.0.1.11" },
      { name: "db", host: "web.internal" },
    ];
    const r = fuzzyFilter(items, "web", (i) => [i.name, i.host]);
    expect(r.map((i) => i.name)).toEqual(["web-01", "db"]);
    expect(fuzzyFilter(items, "  ", (i) => [i.name])).toHaveLength(3);
  });
  it("is case-insensitive", () => {
    expect(fuzzyScore("PROD", "production")).not.toBeNull();
  });
});

describe("quick connect parser", () => {
  it("parses user@host[:port]", () => {
    expect(parseQuickConnect("deploy@10.0.0.5")).toEqual({ user: "deploy", host: "10.0.0.5", port: 22 });
    expect(parseQuickConnect("root@example.com:2222")).toEqual({ user: "root", host: "example.com", port: 2222 });
    expect(parseQuickConnect("  host.local  ")).toEqual({ user: null, host: "host.local", port: 22 });
    expect(parseQuickConnect("host:2200")).toEqual({ user: null, host: "host", port: 2200 });
  });
  it("supports IPv6 and ssh-style input", () => {
    expect(parseQuickConnect("me@[::1]:2022")).toEqual({ user: "me", host: "::1", port: 2022 });
    expect(parseQuickConnect("ssh pi@raspberry -p 2201")).toEqual({ user: "pi", host: "raspberry", port: 2201 });
    expect(parseQuickConnect("CORP\\alice@jump")).toEqual({ user: "CORP\\alice", host: "jump", port: 22 });
  });
  it("rejects invalid targets", () => {
    for (const bad of ["", "@host", "user@", "host:99999", "host:0", "a b", "user@host:port", "bad<script>@x", "-oProxyCommand=x"])
      expect(parseQuickConnect(bad), bad).toBeNull();
  });
});

describe("markdown-lite", () => {
  it("escapes HTML everywhere", () => {
    const out = renderMarkdown('<img src=x onerror=alert(1)> **<b>x</b>** `<script>` "q"');
    expect(out).not.toContain("<img");
    expect(out).not.toContain("<script>");
    expect(out).not.toContain("<b>");
    expect(out).toContain("&lt;img src=x onerror=alert(1)&gt;");
    expect(out).toContain("<strong>&lt;b&gt;x&lt;/b&gt;</strong>");
    expect(out).toContain("<code>&lt;script&gt;</code>");
    expect(out).toContain("&quot;q&quot;");
  });
  it("escapes code blocks and tolerates unclosed fences (streaming)", () => {
    const out = renderMarkdown("```bash\nrm -rf </tmp/x> && echo '1'\n");
    expect(out).toBe('<pre class="md-code"><code>rm -rf &lt;/tmp/x&gt; &amp;&amp; echo &#39;1&#39;</code></pre>');
  });
  it("renders lists, bold, inline code and paragraphs", () => {
    const out = renderMarkdown("Root is at **79%**.\n\n- `/` full\n- docker ok\n\n1. one\n2. two");
    expect(out).toBe(
      "<p>Root is at <strong>79%</strong>.</p><ul><li><code>/</code> full</li><li>docker ok</li></ul><ol><li>one</li><li>two</li></ol>",
    );
  });
  it("does not format inside inline code and never emits attributes from input", () => {
    expect(renderMarkdown("`**not bold**`")).toBe("<p><code>**not bold**</code></p>");
    expect(renderMarkdown('[x](javascript:alert(1)) <a href="j">')).not.toMatch(/<a[\s>]|href="/);
  });
  it("escapeHtml handles all specials", () => {
    expect(escapeHtml(`&<>"'`)).toBe("&amp;&lt;&gt;&quot;&#39;");
  });
});

describe("approval guard", () => {
  it("requires typed connection name for Auto on prod only", () => {
    expect(needsTypedConfirm("prod", "auto")).toBe(true);
    expect(needsTypedConfirm("prod", "confirm_writes")).toBe(false);
    expect(needsTypedConfirm("staging", "auto")).toBe(false);
    expect(canSetApprovalMode("prod", "auto", "web-01", null)).toBe(false);
    expect(canSetApprovalMode("prod", "auto", "web-01", "web-0")).toBe(false);
    expect(canSetApprovalMode("prod", "auto", "web-01", "Web-01")).toBe(false);
    expect(canSetApprovalMode("prod", "auto", "web-01", " web-01 ")).toBe(true);
    expect(canSetApprovalMode("prod", "auto", "", "")).toBe(false);
    expect(canSetApprovalMode("dev", "auto", "dev-box", null)).toBe(true);
  });
});

describe("folder tree", () => {
  const groups = [
    { id: "a", name: "Prod", parent_id: null, order: 0 },
    { id: "b", name: "eu", parent_id: "a", order: 0 },
    { id: "c", name: "Dev", parent_id: null, order: 1 },
  ];
  const conn = (id: string, name: string, group_id: string | null) =>
    ({ id, name, group_id }) as unknown as import("../api/types").Connection;
  it("nests folders, counts and sorts by manual order", () => {
    const { folders, root } = buildTree(groups, [conn("1", "zeta", "b"), conn("2", "alpha", "b"), conn("3", "x", null)], { "1": 0, "2": 1 });
    expect(folders.map((f) => f.group.name)).toEqual(["Prod", "Dev"]);
    expect(folders[0].count).toBe(2);
    expect(folders[0].folders[0].connections.map((c) => c.name)).toEqual(["zeta", "alpha"]);
    expect(root.map((c) => c.id)).toEqual(["3"]);
  });
  it("computes paths and detects cycles", () => {
    expect(groupPath(groups, "b")).toBe("Prod / eu");
    expect(isDescendant(groups, "b", "a")).toBe(true);
    expect(isDescendant(groups, "c", "a")).toBe(false);
  });
});

describe("password strength", () => {
  it("scores length and variety", () => {
    expect(passwordStrength("short")).toBe(0);
    expect(passwordStrength("password1234")).toBe(1);
    expect(passwordStrength("correct horse battery staple 9!")).toBeGreaterThanOrEqual(3);
  });
});

describe("args", () => {
  it("splits like a shell without expansion", () => {
    expect(splitArgs("  --verbose  --name 'my profile' \"say \\\"hi\\\"\" C:\\tools\\x ")).toEqual([
      "--verbose",
      "--name",
      "my profile",
      'say "hi"',
      "C:\\tools\\x",
    ]);
    expect(splitArgs("a''b \"\"")).toEqual(["ab", ""]);
    expect(splitArgs("")).toEqual([]);
    expect(splitArgs("--x 'open")).toBeNull();
    expect(splitArgs('--x "open\\"')).toBeNull();
  });
  it("round-trips through joinArgs", () => {
    const cases = [["--a", "b c"], ["it's", 'say "x"'], ["", "C:\\p\\"], ["mixed '\"\\ "]];
    for (const c of cases) expect(splitArgs(joinArgs(c))).toEqual(c);
  });
});

describe("time", () => {
  const now = Date.UTC(2026, 9, 10, 12, 0, 0);
  it("formats relative times", () => {
    expect(relativeTime(now - 10_000, now)).toBe("just now");
    expect(relativeTime(now + 60_000, now)).toBe("just now");
    expect(relativeTime(now - 5 * 60_000, now)).toBe("5 min ago");
    expect(relativeTime(now - 3 * 3_600_000, now)).toBe("3 h ago");
    expect(relativeTime(now - 26 * 3_600_000, now)).toBe("yesterday");
    expect(relativeTime(now - 3 * 86_400_000, now)).toBe("3 days ago");
    expect(relativeTime(now - 30 * 86_400_000, now)).toBe(new Date(now - 30 * 86_400_000).toLocaleDateString());
  });
  it("formats sizes", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(4300)).toBe("4.2 KB");
    expect(formatBytes(50 * 1024)).toBe("50 KB");
    expect(formatBytes(3 * 1024 * 1024)).toBe("3.0 MB");
  });
});

describe("syncChangedVault", () => {
  const st = (at: number | null, pulled = 0, conflicts = 0, extra: Partial<SyncStatus> = {}): SyncStatus => ({
    can_join: false,
    auto: true,
    syncing: false,
    last_error: null,
    last_sync_ms: at,
    last_report: { pulled, pushed: 0, conflicts, warnings: [] },
    ...extra,
  });
  it("reloads only when a completed sync pulled or merged something", () => {
    expect(syncChangedVault(st(1), st(2, 2))).toBe(true);
    expect(syncChangedVault(st(1), st(2, 0, 1))).toBe(true);
    expect(syncChangedVault(st(1), st(2))).toBe(false);
    expect(syncChangedVault(st(2, 2), st(2, 2)), "same sync reported twice").toBe(false);
    expect(syncChangedVault(st(1), st(1, 2, 0, { syncing: true })), "still running").toBe(false);
    expect(syncChangedVault(st(1), st(2, 2, 0, { last_error: "offline" }))).toBe(false);
    // The listener started after the sync had begun: still reloads.
    expect(syncChangedVault(null, st(2, 2))).toBe(true);
  });
});

describe("snippets", () => {
  it("finds placeholders once, in order, and leaves Go templates alone", () => {
    expect(placeholders("ssh {{ user }}@{{host}} -p {{port}} # {{host}}")).toEqual(["user", "host", "port"]);
    expect(placeholders('docker ps --format "{{.Names}}"')).toEqual([]);
  });
  it("fills placeholders", () => {
    expect(fillPlaceholders("journalctl -u {{service}} -n {{ n }}", { service: "nginx", n: "50" })).toBe("journalctl -u nginx -n 50");
    expect(fillPlaceholders('docker ps --format "{{.Names}}" {{x}}', {})).toBe('docker ps --format "{{.Names}}" ');
  });
  it("types lines with Enter only when running", () => {
    expect(snippetInput("uptime\n", true, false)).toBe("uptime\r");
    expect(snippetInput("cd /tmp\nls", true, false)).toBe("cd /tmp\rls\r");
    expect(snippetInput("uptime", false, false)).toBe("uptime");
  });
  it("never presses Enter when inserting a multi-line snippet", () => {
    expect(snippetInput("cd /tmp\nrm -rf x\n", false, true)).toBe("\x1b[200~cd /tmp\rrm -rf x\x1b[201~");
    expect(snippetInput("cd /tmp\nls", false, false)).toBeNull();
    expect(snippetInput("a\n\x1b[201~rm -rf /", false, true)).toBe("\x1b[200~a\rrm -rf /\x1b[201~");
  });
  it("suggests unique names", () => {
    const names = SUGGESTED_SNIPPETS.map((s) => s.name);
    expect(new Set(names).size).toBe(names.length);
  });
});

describe("files", () => {
  it("formats modes like ls -l", () => {
    expect(modeString(0o40755)).toBe("drwxr-xr-x");
    expect(modeString(0o100600)).toBe("-rw-------");
    expect(modeString(0o120777)).toBe("lrwxrwxrwx");
    expect(modeString(null)).toBe("");
  });
  it("walks paths", () => {
    expect(parentDir("/home/me/logs")).toBe("/home/me");
    expect(parentDir("/home")).toBe("/");
    expect(parentDir("/")).toBe("/");
    expect(joinPath("/", "etc")).toBe("/etc");
    expect(joinPath("/home/me", "a")).toBe("/home/me/a");
  });
  it("accepts only single path segments as names", () => {
    expect(validName("notes.txt")).toBe(true);
    for (const bad of ["", " ", ".", "..", "a/b"]) expect(validName(bad)).toBe(false);
  });
});

describe("s3 helpers", () => {
  const base = { id: "a", name: "x", region: "eu-west-1", access_key_id: "k", secret: null, path_style: false };
  it("describes where an account points", () => {
    expect(s3Where({ ...base, endpoint: "", bucket: null })).toBe("s3.eu-west-1");
    expect(s3Where({ ...base, endpoint: "https://acct.r2.cloudflarestorage.com/", bucket: " logs " })).toBe("logs · acct.r2.cloudflarestorage.com");
  });
  it("recognises presets from the endpoint", () => {
    expect(presetOf({ ...base, endpoint: "", bucket: null }).id).toBe("aws");
    expect(presetOf({ ...base, endpoint: "https://x.r2.cloudflarestorage.com", bucket: null }).id).toBe("r2");
    expect(presetOf({ ...base, endpoint: "http://10.0.0.5:9000", bucket: null }).id).toBe("other");
  });
});

describe("auto-reconnect", () => {
  const info = (p: Partial<SessionInfo>): SessionInfo => ({
    session_id: "s",
    connection_id: "c",
    title: "web",
    env: "none",
    os: null,
    shell: null,
    cwd: null,
    user: null,
    host: null,
    integration: "full",
    state: "disconnected",
    approval_mode: "confirm_all",
    ended: { exit_status: null, error: "Connection reset by peer" },
    ...p,
  });
  it("backs off from 1s to 30s", () => {
    expect([0, 1, 2, 3, 4, 5, 6, 7, 19].map(reconnectDelay)).toEqual([0, 1000, 2000, 4000, 8000, 16000, 30000, 30000, 30000]);
  });
  it("retries the network, never the login or host key", () => {
    expect(retryable({ kind: "network", message: "Connection refused" })).toBe(true);
    expect(retryable({ kind: "io", message: "x" })).toBe(true);
    expect(retryable({ kind: "auth", message: "denied" })).toBe(false);
    expect(retryable({ kind: "host_key_mismatch", message: "x" })).toBe(false);
    expect(retryable(new Error("boom"))).toBe(false);
  });
  it("reconnects only lost SSH sessions that log in without asking", () => {
    expect(shouldAutoReconnect(info({}), "key")).toBe(true);
    expect(shouldAutoReconnect(info({}), "password")).toBe(true);
    expect(shouldAutoReconnect(info({ ended: { exit_status: 0, error: null } }), "key")).toBe(false);
    expect(shouldAutoReconnect(info({}), "prompt")).toBe(false);
    expect(shouldAutoReconnect(info({ connection_id: null }), "key")).toBe(false);
    expect(shouldAutoReconnect(info({ state: "idle" }), "key")).toBe(false);
    expect(shouldAutoReconnect(info({ ended: null }), "key")).toBe(false);
  });
});
