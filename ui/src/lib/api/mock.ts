// In-browser mock backend, used when `window.__TAURI_INTERNALS__` is missing (plain `npm run dev`).
// Seeds realistic data, runs a fake shell per session and a scripted agent run with an approval.
import type { Api, EventMap, EventName, Unlisten } from "./api";
import type {
  ApprovalDecision,
  ApprovalSettings,
  Brain,
  CommandResult,
  Connection,
  Group,
  HarnessProfile,
  ImportPreview,
  Provider,
  SessionInfo,
  Snippet,
  S3Account,
  FileEntry,
  SyncStatus,
  ConflictView,
  SnapshotInfo,
} from "./types";
import { b64ToStr, strToB64 } from "../util/b64";
import { uuid } from "../util/id";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

// ---------- seed data ----------
const g = (name: string, order: number, parent_id: string | null = null): Group => ({ id: uuid(), name, parent_id, order });

function conn(p: Partial<Connection> & Pick<Connection, "name" | "host">): Connection {
  return {
    id: uuid(),
    kind: "ssh",
    group_id: null,
    port: 22,
    user: "deploy",
    auth: { type: "agent" },
    jump: [],
    env: "none",
    tags: [],
    color: null,
    approval_mode: null,
    local_shell: null,
    notes: "",
    ...p,
  };
}

function seed() {
  const prod = g("Production", 0);
  const prodEu = g("eu-west", 0, prod.id);
  const staging = g("Staging", 1);
  const dev = g("Development", 2);
  const personal = g("Personal", 3);
  const groups = [prod, prodEu, staging, dev, personal];
  const bastion = conn({ name: "bastion", host: "bastion.acme.io", env: "prod", group_id: prod.id, tags: ["jump"] });
  const connections: Connection[] = [
    bastion,
    conn({ name: "web-01", host: "10.0.1.11", env: "prod", group_id: prodEu.id, jump: [bastion.id], tags: ["nginx", "web"] }),
    conn({ name: "web-02", host: "10.0.1.12", env: "prod", group_id: prodEu.id, jump: [bastion.id], tags: ["nginx", "web"] }),
    conn({ name: "legacy-db", host: "10.0.2.40", env: "prod", group_id: prodEu.id, user: "postgres", tags: ["postgres"] }),
    conn({ name: "api-staging", host: "api.staging.acme.io", env: "staging", group_id: staging.id, tags: ["api"] }),
    conn({ name: "edge-proxy", host: "edge.staging.acme.io", env: "staging", group_id: staging.id, port: 2222 }),
    conn({ name: "dev-box", host: "192.168.1.50", env: "dev", group_id: dev.id, user: "andrea" }),
    conn({ name: "ci-runner", host: "ci.internal", env: "dev", group_id: dev.id, user: "runner" }),
    conn({ name: "raspberry", host: "pi.local", group_id: personal.id, user: "pi", auth: { type: "prompt" } }),
  ];
  const providers: Provider[] = [
    {
      id: uuid(),
      name: "Anthropic",
      kind: "anthropic",
      preset: "anthropic",
      base_url: "https://api.anthropic.com",
      api_key: uuid(),
      default_model: "claude-sonnet-4-5",
    },
    {
      id: uuid(),
      name: "DeepSeek",
      kind: "open_ai_compat",
      preset: "deep_seek",
      base_url: "https://api.deepseek.com/v1",
      api_key: uuid(),
      default_model: "deepseek-chat",
    },
  ];
  const profiles: HarnessProfile[] = [
    {
      id: uuid(),
      name: "Claude Code (DevOps)",
      harness: "claude_code",
      display: "native",
      server_mode: true,
      skills: ["ops-runbooks"],
      plugins: [],
      extra_args: [],
      model: null,
    },
    {
      id: uuid(),
      name: "Codex",
      harness: "codex",
      display: "sidebar",
      server_mode: true,
      skills: [],
      plugins: [],
      extra_args: [],
      model: null,
    },
  ];
  const snippets: Snippet[] = [
    { id: uuid(), name: "Restart nginx", command: "sudo systemctl restart nginx", description: "", tags: ["nginx"] },
    { id: uuid(), name: "Tail app log", command: "tail -f /var/log/{{app}}/current.log", description: "Follow an app's log", tags: ["logs"] },
  ];
  const s3Accounts: S3Account[] = [
    { id: uuid(), name: "Backups (R2)", endpoint: "https://acct.r2.cloudflarestorage.com", region: "auto", bucket: "backups", access_key_id: "r2-key", secret: uuid(), path_style: false },
    { id: uuid(), name: "AWS production", endpoint: "", region: "eu-west-1", bucket: null, access_key_id: "AKIA…", secret: uuid(), path_style: false },
  ];
  return { groups, connections, providers, profiles, snippets, s3Accounts };
}

// ---------- fake shell ----------
const C = {
  user: "\x1b[38;2;124;156;255m",
  path: "\x1b[38;2;139;143;152m",
  dim: "\x1b[2m",
  reset: "\x1b[0m",
};

interface Shell {
  info: SessionInfo;
  local: boolean;
  line: string;
  cwd: string;
  native?: string;
}

const OS_RELEASE = `PRETTY_NAME="Ubuntu 24.04.1 LTS"
NAME="Ubuntu"
VERSION_ID="24.04"
VERSION="24.04.1 LTS (Noble Numbat)"
VERSION_CODENAME=noble
ID=ubuntu
ID_LIKE=debian
HOME_URL="https://www.ubuntu.com/"`;

const DF = `Filesystem      Size  Used Avail Use% Mounted on
tmpfs           794M  1.2M  793M   1% /run
/dev/nvme0n1p1   78G   61G   17G  79% /
tmpfs           3.9G     0  3.9G   0% /dev/shm
/dev/nvme1n1    492G  301G  166G  65% /var/lib/docker
/dev/nvme0n1p15 105M  6.1M   99M   6% /boot/efi`;

function runFake(sh: Shell, cmd: string): string {
  const [bin, ...args] = cmd.trim().split(/\s+/);
  const host = sh.info.host ?? "localhost";
  switch (bin) {
    case "":
      return "";
    case "ls":
      return sh.local
        ? "    Directory: C:\\Users\\dev\n\nMode   LastWriteTime        Name\n----   -------------        ----\nd----  10/10/2026 09:12      code\nd----  10/09/2026 18:40      Downloads\n-a---  10/01/2026 11:02      notes.md"
        : `${C.user}app${C.reset}  ${C.user}backups${C.reset}  ${C.user}logs${C.reset}  docker-compose.yml  deploy.sh  README.md`;
    case "df":
      return DF;
    case "uptime":
      return ` 14:32:07 up 41 days,  3:12,  2 users,  load average: 0.42, 0.37, 0.31`;
    case "cat":
      if (args[0] === "/etc/os-release") return OS_RELEASE;
      return `cat: ${args[0] ?? ""}: No such file or directory`;
    case "whoami":
      return sh.info.user ?? "dev";
    case "hostname":
      return host;
    case "pwd":
      return sh.cwd === "~" ? `/home/${sh.info.user}` : sh.cwd;
    case "echo":
      return args.join(" ");
    case "clear":
      return "\x1b[2J\x1b[H";
    case "help":
      return `${C.dim}mock shell: ls, df -h, uptime, cat /etc/os-release, whoami, hostname, pwd, echo, clear${C.reset}`;
    case "free":
      return "               total        used        free      shared  buff/cache   available\nMem:           7.8Gi       3.1Gi       1.2Gi       112Mi       3.5Gi       4.3Gi\nSwap:          2.0Gi          0B       2.0Gi";
    default:
      return `${sh.local ? "" : "bash: "}${bin}: command not found`;
  }
}

function prompt(sh: Shell): string {
  if (sh.native) return "";
  if (sh.local) return `${C.path}PS C:\\Users\\dev>${C.reset} `;
  const host = /^[\d.]+$/.test(sh.info.host ?? "") ? sh.info.title : sh.info.host?.split(".")[0];
  return `${C.user}${sh.info.user}@${host}${C.reset}:${C.path}${sh.cwd}${C.reset}$ `;
}

// ---------- mock API ----------
export function createMockApi(): Api {
  const db = seed();
  const vault = { initialized: false, locked: true, password: "" };
  const shells = new Map<string, Shell>();
  const listeners = new Map<string, Set<(p: unknown) => void>>();
  const pendingHostKeys = new Map<string, (ok: boolean) => void>();
  const pendingApprovals = new Map<string, (d: ApprovalDecision) => void>();
  const cancelled = new Set<string>();
  const trusted = new Set<string>();
  const previews = new Map<string, ImportPreview>();
  let approvalSettings: ApprovalSettings = {
    default_mode: "confirm_all",
    deny_list: ["shutdown", "reboot"],
  };
  let sync: SyncStatus = { config: null, last_sync_ms: null, last_report: null, can_join: false, auto: true, syncing: false, last_error: null };
  const hour = 3_600_000;
  const snapName = (ms: number) => `${String(ms).padStart(15, "0")}-${Math.random().toString(16).slice(2, 10)}.snap`;
  let snapshots: SnapshotInfo[] = [3, 26, 50].map((h) => {
    const created_ms = Date.now() - h * hour;
    return { name: snapName(created_ms), created_ms, size: 4096 + h * 37 };
  });
  let conflicts: ConflictView[] = [
    {
      file_id: "9f2c1d0e8b7a6f5e4d3c2b1a09f8e7d6",
      recorded_at_ms: Date.now() - 2 * hour,
      item_type: "connection",
      label: "web-01",
      loser_ms: Date.now() - 3 * hour,
      loser_device: "a1b2c3d4-0000-4000-8000-000000000001",
      loser_deleted: false,
      winner_ms: Date.now() - 2.5 * hour,
    },
    {
      file_id: "0a1b2c3d4e5f60718293a4b5c6d7e8f9",
      recorded_at_ms: Date.now() - 30 * hour,
      item_type: "secret",
      label: "db password",
      loser_ms: Date.now() - 31 * hour,
      loser_device: "a1b2c3d4-0000-4000-8000-000000000002",
      loser_deleted: true,
      winner_ms: Date.now() - 30.5 * hour,
    },
  ];
  const syncEvent = () => emit("sync://status", structuredClone(sync));
  const authPrompts = new Map<string, (answers: string[] | null) => void>();
  // ---- fake remote file system (one for all SSH sessions) ----
  type FsNode = { kind: "dir" | "file" | "symlink"; size: number; modified: number; permissions: number; link?: boolean };
  const files = new Map<string, FsNode>();
  const parentOf = (p: string) => p.replace(/\/[^/]+$/, "") || "/";
  const baseName = (p: string) => p.split(/[\\/]/).pop() ?? p;
  function seedFiles(home: string) {
    if (files.has(home)) return;
    const now = Math.floor(Date.now() / 1000);
    const dir = (p: string, age = 3600) => files.set(p, { kind: "dir", size: 4096, modified: now - age, permissions: 0o40755 });
    const file = (p: string, size: number, age = 7200, mode = 0o100644) => files.set(p, { kind: "file", size, modified: now - age, permissions: mode });
    for (const p of ["/", "/home", home]) dir(p, 86400 * 30);
    dir(`${home}/app`);
    dir(`${home}/logs`, 600);
    file(`${home}/.bashrc`, 3771, 86400 * 90);
    file(`${home}/deploy.sh`, 1290, 86400, 0o100755);
    file(`${home}/app/docker-compose.yml`, 2048);
    file(`${home}/app/.env`, 412, 86400 * 3, 0o100600);
    file(`${home}/logs/app.log`, 18_400_000, 60);
    file(`${home}/logs/app.log.1.gz`, 2_100_000, 86400);
    // Symlinks carry their target's kind, as the backend reports them.
    files.set(`${home}/current`, { kind: "dir", size: 4096, modified: now - 600, permissions: 0o120777, link: true });
    files.set(`${home}/latest.log`, { kind: "file", size: 18_400_000, modified: now - 600, permissions: 0o120777, link: true });
    files.set(`${home}/old-release`, { kind: "symlink", size: 0, modified: now - 86400 * 40, permissions: 0o120777, link: true });
  }
  function sshShell(session_id: string) {
    const sh = shells.get(session_id);
    if (!sh) throw fail("not_found", `session ${session_id}`);
    if (sh.local) throw fail("invalid", "files are only available for SSH sessions");
    const home = `/home/${sh.info.user}`;
    seedFiles(home);
    return { sh, home };
  }
  async function transfer(transfer_id: string, total: number) {
    for (let i = 0; i <= 10; i++) {
      emit("transfer://progress", { transfer_id, done: Math.round((total * i) / 10), total });
      await sleep(60);
    }
  }
  // ---- fake object store: bucket -> key -> object (shared by every account) ----
  type S3Obj = { size: number; modified: number };
  const buckets = new Map<string, { created: number; objects: Map<string, S3Obj> }>();
  {
    const now = Math.floor(Date.now() / 1000);
    const bucket = (name: string, keys: [string, number, number][]) =>
      buckets.set(name, { created: now - 86400 * 400, objects: new Map(keys.map(([k, size, age]) => [k, { size, modified: now - age }])) });
    bucket("backups", [
      ["db/", 0, 86400],
      ["db/2026-10-10.sql.gz", 48_200_000, 86400],
      ["db/2026-10-11.sql.gz", 48_900_000, 3600],
      ["site/", 0, 86400 * 9],
      ["site/www.tar.zst", 12_400_000, 86400 * 9],
      ["README.txt", 512, 86400 * 60],
    ]);
    bucket("static-assets", [["logo.svg", 4096, 86400 * 30], ["css/app.css", 22_000, 7200]]);
  }
  function s3Account(id: string) {
    const a = db.s3Accounts.find((x) => x.id === id);
    if (!a) throw fail("not_found", `S3 account ${id}`);
    return a;
  }
  function s3Split(path: string) {
    const [bucket = "", ...rest] = path.split("/").filter(Boolean);
    return { bucket, key: rest.join("/") };
  }
  function s3Bucket(name: string) {
    const b = buckets.get(name);
    if (!b) throw fail("not_found", "NoSuchBucket: The specified bucket does not exist");
    return b;
  }
  function s3Object(path: string) {
    const { bucket, key } = s3Split(path);
    if (!bucket) throw fail("invalid", "open a bucket first");
    if (!key) throw fail("invalid", "buckets are created and deleted in your provider's console");
    return { b: s3Bucket(bucket), bucket, key };
  }
  const fail = (kind: string, message: string) => Object.assign(new Error(message), { kind, message });
  let githubPolls = 0;

  function emit<E extends EventName>(event: E, payload: EventMap[E]) {
    listeners.get(event)?.forEach((cb) => cb(payload));
  }
  const write = (sid: string, s: string) => emit("session://data", { session_id: sid, data_b64: strToB64(s) });
  const setState = (sh: Shell, state: SessionInfo["state"]) => {
    sh.info = { ...sh.info, state };
    emit("session://state", sh.info);
  };

  function newSession(p: Partial<SessionInfo>, local: boolean): Shell {
    const info: SessionInfo = {
      session_id: uuid(),
      connection_id: null,
      title: "local",
      env: "none",
      os: null,
      shell: null,
      cwd: "~",
      user: null,
      host: null,
      integration: "unknown",
      state: "connecting",
      approval_mode: approvalSettings.default_mode,
      ...p,
    };
    const sh: Shell = { info, local, line: "", cwd: "~" };
    shells.set(info.session_id, sh);
    return sh;
  }

  async function boot(sh: Shell, c: Connection | null) {
    await sleep(60);
    if (c) {
      write(sh.info.session_id, `${C.dim}Connecting to ${c.user}@${c.host}:${c.port}${c.jump.length ? " via bastion" : ""}…${C.reset}\r\n`);
      const hostKey = `${c.host}:${c.port}`;
      const kind = c.name === "legacy-db" ? "changed" : c.name === "edge-proxy" && !trusted.has(hostKey) ? "new" : null;
      if (kind) {
        const request_id = uuid();
        const ok = await new Promise<boolean>((res) => {
          pendingHostKeys.set(request_id, res);
          emit("session://host_key", {
            request_id,
            host: hostKey,
            kind,
            key_type: "ssh-ed25519",
            fingerprint: "SHA256:q3Vx8f2nK0tL9mXcE4pR7bZs1YhUo6WjA5dQiN+gTkc",
            old_fingerprint: kind === "changed" ? "SHA256:Hb2K9sPq4Lw7Rt1ZxV0cN8mJfY3aGd6UeO5iS+kQrTo" : undefined,
          });
        });
        if (!ok) {
          write(sh.info.session_id, `\x1b[38;2;232;106;106mHost key rejected. Connection closed.${C.reset}\r\n`);
          setState(sh, "disconnected");
          return;
        }
        trusted.add(hostKey);
      }
      await sleep(350);
      write(
        sh.info.session_id,
        `Welcome to Ubuntu 24.04.1 LTS (GNU/Linux 6.8.0-45-generic x86_64)\r\n\r\n` +
          `  System load:  0.42             Processes:             187\r\n` +
          `  Usage of /:   79.1% of 77.4GB   Users logged in:       1\r\n` +
          `  Memory usage: 41%              IPv4 address for eth0: ${c.host}\r\n\r\n` +
          `${C.dim}Last login: Fri Oct  9 18:22:41 2026 from 10.0.0.4${C.reset}\r\n`,
      );
      sh.info = { ...sh.info, os: "Ubuntu 24.04", shell: "bash", integration: "full" };
    } else if (sh.native) {
      write(sh.info.session_id, sh.native);
      sh.info = { ...sh.info, integration: "unknown" };
      setState(sh, "full_screen");
      return;
    } else {
      write(sh.info.session_id, `PowerShell 7.5.0\r\n${C.dim}mock local shell — type "help"${C.reset}\r\n\r\n`);
      sh.info = { ...sh.info, os: "Windows 11", shell: "pwsh", integration: "full" };
    }
    setState(sh, "idle");
    write(sh.info.session_id, prompt(sh));
  }

  async function execLine(sh: Shell, cmd: string, slow = false): Promise<{ out: string; ms: number }> {
    const sid = sh.info.session_id;
    const t0 = performance.now();
    if (slow) await sleep(450);
    const out = runFake(sh, cmd);
    if (out) write(sid, out.replace(/\n/g, "\r\n") + (out.endsWith("\x1b[H") ? "" : "\r\n"));
    write(sid, prompt(sh));
    return { out, ms: Math.round(performance.now() - t0) + (slow ? 0 : 3) };
  }

  function stripAnsi(s: string) {
    // eslint-disable-next-line no-control-regex
    return s.replace(/\x1b\[[0-9;]*[A-Za-z]/g, "");
  }

  // ---- scripted agent run ----
  async function agentRun(run_id: string, sh: Shell, brain: Brain, message: string) {
    const session_id = sh.info.session_id;
    const ev = (event: EventMap["agent://event"]["event"]) => emit("agent://event", { run_id, session_id, event });
    const actor =
      brain.type === "harness"
        ? db.profiles.find((p) => p.id === brain.profile)?.name ?? "Harness"
        : `${db.providers.find((p) => p.id === brain.provider)?.name ?? "Built-in"} · ${brain.model}`;
    const stream = async (text: string) => {
      for (const chunk of text.match(/.{1,6}/gs) ?? []) {
        if (cancelled.has(run_id)) return false;
        ev({ type: "text_delta", text: chunk });
        await sleep(18);
      }
      return true;
    };
    const wantsUptime = /uptime|load/i.test(message);
    const command = wantsUptime ? "uptime" : "df -h";
    const host = sh.info.host ?? "this host";

    if (!(await stream(`I'll check ${wantsUptime ? "the load" : "disk usage"} on **${host}** first.\n\n`))) return finishCancelled();
    const call_id = uuid();
    ev({ type: "tool_call", call_id, name: "run_command", input: { session_id, command, reason: wantsUptime ? "Read load averages" : "See which filesystems are filling up" } });
    await sleep(250);

    let finalCommand = command;
    if (sh.info.approval_mode !== "auto") {
      const approval_id = uuid();
      const req = {
        approval_id,
        session_id,
        actor_label: actor,
        command,
        reason: wantsUptime ? "Read load averages" : "See which filesystems are filling up",
        risk: "read" as const,
        policy_note: sh.info.approval_mode === "confirm_all" ? "Approval mode: ask for every command" : null,
      };
      if (sh.info.approval_mode === "confirm_all") {
        ev({ type: "approval_requested", ...req });
        const decision = await new Promise<ApprovalDecision>((res) => pendingApprovals.set(approval_id, res));
        ev({ type: "approval_resolved", approval_id, decision });
        if (decision.type === "deny") {
          const result: CommandResult = {
            state: "denied",
            exit_code: null,
            output: "",
            truncated: false,
            output_id: null,
            duration_ms: 0,
            command,
            note: decision.message,
          };
          ev({ type: "tool_result", call_id, output: result, is_error: false });
          await stream("Okay, I won't run that. Tell me what you'd prefer and I'll adjust.");
          ev({ type: "usage", input_tokens: 1840, output_tokens: 96, cost_usd: 0.0068 });
          ev({ type: "done" });
          return;
        }
        if (decision.type === "approve_edited") finalCommand = decision.command;
      }
    }
    if (cancelled.has(run_id)) return finishCancelled();

    // Type the command into the live terminal.
    setState(sh, "agent_running");
    emit("session://agent_mark", { session_id, command: finalCommand, actor_label: actor });
    for (const ch of finalCommand) {
      if (cancelled.has(run_id)) break;
      write(session_id, ch);
      await sleep(28);
    }
    write(session_id, "\r\n");
    const { out, ms } = await execLine(sh, finalCommand, true);
    setState(sh, "idle");
    const result: CommandResult = {
      state: cancelled.has(run_id) ? "interrupted" : "completed",
      exit_code: /command not found/.test(out) ? 127 : 0,
      output: stripAnsi(out),
      truncated: false,
      output_id: uuid(),
      duration_ms: ms,
      command: finalCommand,
      note: null,
    };
    ev({ type: "tool_result", call_id, output: result, is_error: false });
    if (cancelled.has(run_id)) return finishCancelled();
    await sleep(200);
    await stream(
      wantsUptime
        ? "Load is low and steady:\n\n- **1 min:** 0.42\n- **15 min:** 0.31\n- Up for 41 days\n\nNothing to worry about."
        : "Root is at **79%** (17G free) — worth watching but not urgent.\n\n- `/` — 61G of 78G used\n- `/var/lib/docker` — 65%, 166G free\n\nTo see what's growing on root, I'd run:\n\n```bash\nsudo du -xh / --max-depth=2 | sort -rh | head -15\n```",
    );
    ev({ type: "usage", input_tokens: 2412, output_tokens: 214, cost_usd: 0.0105 });
    ev({ type: "done" });

    function finishCancelled() {
      ev({ type: "error", message: "Run cancelled — you took over the terminal.", retryable: false });
      ev({ type: "done" });
    }
  }

  const api: Api = {
    backend: "mock",
    async vaultStatus() {
      return { initialized: vault.initialized, locked: vault.locked, needs_secret_key: false };
    },
    async vaultCreate(password) {
      await sleep(400);
      vault.initialized = true;
      vault.locked = false;
      vault.password = password;
      const groups = Array.from({ length: 6 }, () => Math.random().toString(36).slice(2, 7).toUpperCase().padEnd(5, "X"));
      return { secret_key: `A3-${groups.join("-")}` };
    },
    async vaultUnlock(password) {
      await sleep(300);
      if (vault.password && password !== vault.password) throw fail("auth", "Wrong master password.");
      vault.locked = false;
    },
    async vaultLock() {
      vault.locked = true;
    },
    async appVersion() {
      return "0.1.0";
    },
    async checkForUpdate() {
      await sleep(400);
      // `?update` in the URL simulates a published update.
      const offer = typeof location !== "undefined" && new URLSearchParams(location.search).has("update");
      return offer ? { version: "0.1.42", current_version: "0.1.0", notes: "Test build", date: new Date().toISOString() } : null;
    },
    async installUpdate(progress) {
      for (let i = 0; i <= 10; i++) {
        progress(i / 10);
        await sleep(80);
      }
    },
    async restartApp() {
      location.reload();
    },
    async openUrl(url) {
      if (!/^https:\/\//.test(url)) throw new Error("only https links can be opened");
      if (typeof window !== "undefined") window.open(url, "_blank", "noopener,noreferrer");
    },
    async saveEmergencyKit(contents) {
      // In a real browser a blob download works; the desktop webview needs the backend command.
      if (typeof document !== "undefined" && typeof URL.createObjectURL === "function") {
        const url = URL.createObjectURL(new Blob([contents], { type: "text/plain" }));
        const a = document.createElement("a");
        a.href = url;
        a.download = "Handrail Emergency Kit.txt";
        a.click();
        setTimeout(() => URL.revokeObjectURL(url), 1000);
      }
      return "~/Downloads/Handrail Emergency Kit.txt";
    },

    async listConnections() {
      return structuredClone(db.connections);
    },
    async saveConnection(c) {
      const i = db.connections.findIndex((x) => x.id === c.id);
      if (i >= 0) db.connections[i] = structuredClone(c);
      else db.connections.push(structuredClone(c));
      return c;
    },
    async deleteConnection(id) {
      db.connections = db.connections.filter((c) => c.id !== id);
    },
    async listGroups() {
      return structuredClone(db.groups);
    },
    async saveGroup(group) {
      const i = db.groups.findIndex((x) => x.id === group.id);
      if (i >= 0) db.groups[i] = structuredClone(group);
      else db.groups.push(structuredClone(group));
      return group;
    },
    async deleteGroup(id) {
      db.groups = db.groups.filter((x) => x.id !== id);
      for (const gr of db.groups) if (gr.parent_id === id) gr.parent_id = null;
      for (const c of db.connections) if (c.group_id === id) c.group_id = null;
    },
    async saveSecret(secret) {
      return secret.id;
    },

    async openSession(connection_id, _cols, _rows, prompt_ref) {
      const c = db.connections.find((x) => x.id === connection_id);
      if (!c) throw fail("not_found", "Connection not found.");
      await sleep(250);
      if (c.auth.type === "prompt") {
        if (!prompt_ref) throw fail("auth", `${c.name} has no saved password or key, so only you can log in to it`);
        // Like the real login: three tries; an empty password is "wrong".
        let ok = false;
        for (let i = 0; i < 3 && !ok; i++) {
          const answers = await new Promise<string[] | null>((resolve) => {
            const request_id = uuid();
            authPrompts.set(request_id, resolve);
            emit("session://auth_prompt", {
              request_id,
              prompt_ref,
              name: "",
              instructions: i ? "Permission denied, please try again." : "",
              prompts: [{ prompt: `${c.user}@${c.host}'s password: `, echo: false }],
            });
          });
          if (!answers) throw fail("auth", "login cancelled");
          ok = !!answers[0];
        }
        if (!ok) throw fail("auth", `password authentication failed for ${c.user}@${c.host}:${c.port}`);
      }
      const mode = c.approval_mode ?? approvalSettings.default_mode;
      const sh = newSession(
        { connection_id, title: c.name, env: c.env, user: c.user, host: c.host, cwd: "~", approval_mode: c.env === "prod" && mode === "auto" ? "confirm_all" : mode },
        false,
      );
      void boot(sh, c);
      return sh.info;
    },
    async answerAuthPrompt(request_id, answers) {
      authPrompts.get(request_id)?.(answers);
      authPrompts.delete(request_id);
    },
    async openLocalSession(shell) {
      const sh = newSession({ title: shell ?? "pwsh", user: "dev", host: "localhost" }, true);
      void boot(sh, null);
      return sh.info;
    },
    async sessionWrite(session_id, data_b64) {
      const sh = shells.get(session_id);
      if (!sh || sh.info.state === "disconnected" || sh.native) return;
      if (sh.info.state === "agent_running") return; // agent owns the line (real core pauses the agent)
      const data = b64ToStr(data_b64);
      for (const ch of data) {
        if (ch === "\r") {
          write(session_id, "\r\n");
          const line = sh.line;
          sh.line = "";
          await execLine(sh, line);
        } else if (ch === "\x7f" || ch === "\b") {
          if (sh.line) {
            sh.line = sh.line.slice(0, -1);
            write(session_id, "\b \b");
          }
        } else if (ch === "\x03") {
          sh.line = "";
          write(session_id, "^C\r\n" + prompt(sh));
        } else if (ch === "\x0c") {
          write(session_id, "\x1b[2J\x1b[H" + prompt(sh) + sh.line);
        } else if (ch >= " " && ch !== "\x1b") {
          sh.line += ch;
          write(session_id, ch);
        }
      }
    },
    async sessionResize() {},
    async closeSession(session_id) {
      shells.delete(session_id);
    },
    async hostKeyDecision(request_id, accept) {
      pendingHostKeys.get(request_id)?.(accept);
      pendingHostKeys.delete(request_id);
    },

    async approvalDecide(approval_id, decision) {
      pendingApprovals.get(approval_id)?.(decision);
      pendingApprovals.delete(approval_id);
    },
    async setApprovalMode(session_id, mode) {
      const sh = shells.get(session_id);
      if (!sh) return;
      sh.info = { ...sh.info, approval_mode: mode };
      emit("session://state", sh.info);
    },
    async getApprovalSettings() {
      return structuredClone(approvalSettings);
    },
    async saveApprovalSettings(s) {
      approvalSettings = structuredClone(s);
    },

    async sftpHome(session_id) {
      return sshShell(session_id).home;
    },
    async sftpList(session_id, path) {
      sshShell(session_id);
      const node = files.get(path);
      if (!node) throw fail("not_found", `No such file: ${path}`);
      if (node.kind !== "dir") throw fail("invalid", `${path} is not a folder`);
      return [...files.entries()]
        .filter(([p]) => p !== path && parentOf(p) === path)
        .map(([p, n]) => ({ name: baseName(p), path: p, kind: n.kind, link: !!n.link, size: n.size, modified: n.modified, permissions: n.permissions }))
        .sort((a, b) => Number(b.kind === "dir") - Number(a.kind === "dir") || a.name.localeCompare(b.name));
    },
    async sftpMkdir(session_id, path) {
      sshShell(session_id);
      if (files.has(path)) throw fail("other", "SFTP: already exists");
      if (files.get(parentOf(path))?.kind !== "dir") throw fail("not_found", "No such file");
      files.set(path, { kind: "dir", size: 4096, modified: Math.floor(Date.now() / 1000), permissions: 0o40755 });
    },
    async sftpRename(session_id, from, to) {
      sshShell(session_id);
      if (!files.has(from)) throw fail("not_found", "No such file");
      if (files.has(to)) throw fail("other", "SFTP: already exists");
      for (const [p, n] of [...files.entries()])
        if (p === from || p.startsWith(from + "/")) {
          files.delete(p);
          files.set(to + p.slice(from.length), n);
        }
    },
    async sftpRemove(session_id, path, is_dir) {
      sshShell(session_id);
      const n = files.get(path);
      if (!n) throw fail("not_found", "No such file");
      if (is_dir && n.link) throw fail("other", "SFTP: Failure (not a folder)");
      if (is_dir && [...files.keys()].some((p) => parentOf(p) === path && p !== path)) throw fail("other", "SFTP: Failure (folder not empty)");
      files.delete(path);
    },
    async sftpDownload(session_id, path, transfer_id) {
      sshShell(session_id);
      const n = files.get(path);
      if (!n || n.kind !== "file") throw fail("not_found", "No such file");
      await transfer(transfer_id, n.size);
      return `~/Downloads/${baseName(path)}`;
    },
    async sftpUpload(session_id, localPath, dir, transfer_id) {
      sshShell(session_id);
      const size = 250_000 + Math.floor(Math.random() * 4_000_000);
      await transfer(transfer_id, size);
      const remote = `${dir === "/" ? "" : dir}/${baseName(localPath)}`;
      files.set(remote, { kind: "file", size, modified: Math.floor(Date.now() / 1000), permissions: 0o100644 });
      return remote;
    },
    async pickFiles() {
      return ["/Users/dev/Desktop/report.pdf"];
    },

    async listS3Accounts() {
      return structuredClone(db.s3Accounts);
    },
    async saveS3Account(a, secretKey) {
      if (!a.name.trim() || !a.access_key_id.trim()) throw fail("invalid", "an S3 account needs a name and an access key ID");
      const saved = { ...structuredClone(a), secret: secretKey?.trim() ? (a.secret ?? uuid()) : a.secret };
      if (!saved.secret) throw fail("invalid", "an S3 account needs a secret access key");
      const i = db.s3Accounts.findIndex((x) => x.id === a.id);
      if (i >= 0) db.s3Accounts[i] = saved;
      else db.s3Accounts.push(saved);
      return structuredClone(saved);
    },
    async deleteS3Account(id) {
      db.s3Accounts = db.s3Accounts.filter((a) => a.id !== id);
    },
    async s3Home(accountId) {
      const b = s3Account(accountId).bucket?.trim();
      return b ? `/${b}` : "/";
    },
    async s3List(accountId, path) {
      s3Account(accountId);
      await sleep(80);
      const { bucket, key } = s3Split(path);
      if (!bucket)
        return [...buckets.entries()].map(([name, b]) => ({ name, path: `/${name}`, kind: "dir" as const, link: false, size: 0, modified: b.created, permissions: null }));
      const prefix = key ? key + "/" : "";
      const folders = new Set<string>();
      const out: FileEntry[] = [];
      for (const [k, o] of s3Bucket(bucket).objects) {
        if (!k.startsWith(prefix) || k === prefix) continue;
        const rest = k.slice(prefix.length);
        const slash = rest.indexOf("/");
        if (slash >= 0) folders.add(rest.slice(0, slash));
        else out.push({ name: rest, path: `/${bucket}/${k}`, kind: "file", link: false, size: o.size, modified: o.modified, permissions: null });
      }
      const dirs = [...folders].sort().map((name) => ({ name, path: `/${bucket}/${prefix}${name}`, kind: "dir" as const, link: false, size: 0, modified: null, permissions: null }));
      return [...dirs, ...out.sort((a, b) => a.name.localeCompare(b.name))];
    },
    async s3Mkdir(accountId, path) {
      s3Account(accountId);
      const { b, key } = s3Object(path);
      b.objects.set(key + "/", { size: 0, modified: Math.floor(Date.now() / 1000) });
    },
    async s3Rename(accountId, from, to, isDir) {
      s3Account(accountId);
      if (isDir) throw fail("invalid", "S3 folders cannot be renamed: every object inside would have to be copied");
      const src = s3Object(from);
      const dst = s3Object(to);
      if (src.bucket !== dst.bucket) throw fail("invalid", "a rename stays in the same bucket");
      const o = src.b.objects.get(src.key);
      if (!o) throw fail("not_found", "NoSuchKey: The specified key does not exist.");
      src.b.objects.delete(src.key);
      src.b.objects.set(dst.key, o);
    },
    async s3Remove(accountId, path, isDir) {
      s3Account(accountId);
      const { b, key } = s3Object(path);
      if (isDir) {
        if ([...b.objects.keys()].some((k) => k.startsWith(key + "/") && k !== key + "/")) throw fail("invalid", "the folder is not empty");
        b.objects.delete(key + "/");
      } else b.objects.delete(key);
    },
    async s3Download(accountId, path, transfer_id) {
      s3Account(accountId);
      const { b, key } = s3Object(path);
      const o = b.objects.get(key);
      if (!o) throw fail("not_found", "NoSuchKey: The specified key does not exist.");
      await transfer(transfer_id, o.size);
      return `~/Downloads/${baseName(key)}`;
    },
    async s3Upload(accountId, localPath, dir, transfer_id) {
      s3Account(accountId);
      const { bucket, key } = s3Split(dir);
      if (!bucket) throw fail("invalid", "open a bucket first");
      const b = s3Bucket(bucket);
      const size = 250_000 + Math.floor(Math.random() * 4_000_000);
      await transfer(transfer_id, size);
      const k = (key ? key + "/" : "") + baseName(localPath);
      b.objects.set(k, { size, modified: Math.floor(Date.now() / 1000) });
      return `/${bucket}/${k}`;
    },

    async listSnippets() {
      return structuredClone(db.snippets);
    },
    async saveSnippet(s) {
      if (!s.name.trim() || !s.command.trim()) throw fail("invalid", "a snippet needs a name and a command");
      const i = db.snippets.findIndex((x) => x.id === s.id);
      if (i >= 0) db.snippets[i] = structuredClone(s);
      else db.snippets.push(structuredClone(s));
      return structuredClone(s);
    },
    async deleteSnippet(id) {
      db.snippets = db.snippets.filter((s) => s.id !== id);
    },

    async listProviders() {
      return structuredClone(db.providers);
    },
    async saveProvider(p, apiKey) {
      const saved = { ...p, api_key: apiKey ? uuid() : p.api_key };
      const i = db.providers.findIndex((x) => x.id === p.id);
      if (i >= 0) db.providers[i] = saved;
      else db.providers.push(saved);
      return saved;
    },
    async deleteProvider(id) {
      db.providers = db.providers.filter((p) => p.id !== id);
    },
    async listModels(provider_id) {
      await sleep(200);
      const p = db.providers.find((x) => x.id === provider_id);
      const byPreset: Record<string, string[]> = {
        anthropic: ["claude-opus-4-1", "claude-sonnet-4-5", "claude-haiku-4-5"],
        open_ai: ["gpt-5", "gpt-5-mini", "gpt-4.1"],
        deep_seek: ["deepseek-chat", "deepseek-reasoner"],
        open_router: ["anthropic/claude-sonnet-4.5", "openai/gpt-5", "qwen/qwen3-coder"],
        ollama: ["qwen2.5-coder:14b", "llama3.1:8b"],
        lm_studio: ["local-model"],
      };
      return byPreset[p?.preset ?? ""] ?? [p?.default_model ?? "model"];
    },
    async testProvider(provider_id) {
      await sleep(600);
      const p = db.providers.find((x) => x.id === provider_id);
      return p?.api_key || p?.preset === "ollama" || p?.preset === "lm_studio"
        ? { ok: true, message: "Connected · 142 ms" }
        : { ok: false, message: "401 Unauthorized — check the API key." };
    },
    async detectHarnesses() {
      await sleep(250);
      return [
        { kind: "claude_code", path: "C:\\Users\\dev\\.local\\bin\\claude.exe", version: "2.0.14", logged_in: true },
        { kind: "codex", path: "C:\\Users\\dev\\AppData\\Roaming\\npm\\codex.cmd", version: "0.46.0", logged_in: false },
        { kind: "open_code", path: "C:\\Users\\dev\\.opencode\\bin\\opencode.exe", version: "0.15.2", logged_in: true },
      ];
    },
    async listHarnessProfiles() {
      return structuredClone(db.profiles);
    },
    async saveHarnessProfile(p) {
      const i = db.profiles.findIndex((x) => x.id === p.id);
      if (i >= 0) db.profiles[i] = structuredClone(p);
      else db.profiles.push(structuredClone(p));
      return p;
    },
    async deleteHarnessProfile(id) {
      db.profiles = db.profiles.filter((p) => p.id !== id);
    },
    async listHarnessExtensions() {
      return {
        skills: [
          { name: "ops-runbooks", path: "~/.claude/skills/ops-runbooks" },
          { name: "postgres-tuning", path: "~/.claude/skills/postgres-tuning" },
          { name: "incident-notes", path: "~/.claude/skills/incident-notes" },
        ],
        plugins: [
          { name: "k8s-helper", path: "~/.claude/plugins/k8s-helper" },
          { name: "log-search", path: "~/.claude/plugins/log-search" },
        ],
      };
    },
    async agentSend(session_id, brain, message) {
      const sh = shells.get(session_id);
      if (!sh) throw fail("not_found", "Session not found.");
      const run_id = uuid();
      void agentRun(run_id, sh, brain, message);
      return { run_id };
    },
    async agentCancel(run_id) {
      cancelled.add(run_id);
      for (const [id, res] of pendingApprovals) {
        res({ type: "deny", message: "cancelled" });
        pendingApprovals.delete(id);
      }
    },
    async harnessOpenNative(_session_id, profile_id) {
      const p = db.profiles.find((x) => x.id === profile_id);
      const sh = newSession({ title: p?.name ?? "Claude Code", user: "dev", host: "localhost" }, true);
      sh.native =
        `\x1b[38;2;217;140;100m╭──────────────────────────────────────────╮\r\n` +
        `│ ✻ Welcome to Claude Code                  │\r\n` +
        `│   /help for help · cwd: ~/handrail/prof  │\r\n` +
        `╰──────────────────────────────────────────╯${C.reset}\r\n\r\n` +
        `${C.dim}  MCP: handrail connected (7 tools)${C.reset}\r\n\r\n> `;
      void boot(sh, null);
      return sh.info;
    },
    async harnessLogin(kind) {
      const sh = newSession({ title: `${kind} login`, user: "dev", host: "localhost" }, true);
      sh.native = `${C.dim}$ ${kind === "claude_code" ? "claude /login" : kind === "codex" ? "codex login" : "opencode auth login"}${C.reset}\r\nOpening your browser to sign in…\r\n`;
      void boot(sh, null);
      return sh.info;
    },

    async importDetect() {
      await sleep(300);
      return [
        { id: "ssh_config", tool: "OpenSSH", path: "~/.ssh/config", count_hint: 6 },
        { id: "putty", tool: "PuTTY", path: "HKCU\\Software\\SimonTatham\\PuTTY\\Sessions", count_hint: 3 },
        { id: "mobaxterm", tool: "MobaXterm", path: "%APPDATA%\\MobaXterm\\MobaXterm.ini", count_hint: 4 },
      ];
    },
    async importPreview(args) {
      await sleep(300);
      if (args.file_path && !args.file_path.startsWith("/") && !/^[A-Za-z]:[\\/]/.test(args.file_path)) {
        throw fail("io", `${args.file_path}: No such file or directory`);
      }
      // Pretend every mRemoteNG file uses the custom master password "secret".
      if (args.tool === "mremoteng" && args.mremoteng_password !== "secret") {
        throw fail("auth", "mRemoteNG data could not be decrypted (wrong master password?)");
      }
      const gr = g("Imported · OpenSSH", 10);
      const sub = g("homelab", 0, gr.id);
      const existing = db.connections[1];
      const items = [
        conn({ name: "web-01", host: existing.host, user: existing.user, group_id: gr.id }),
        conn({ name: "nas", host: "nas.home", user: "admin", group_id: sub.id }),
        conn({ name: "router", host: "192.168.1.1", user: "root", group_id: sub.id }),
        conn({ name: "gitlab", host: "git.acme.io", user: "git", group_id: gr.id }),
        conn({ name: "vpn", host: "vpn.acme.io", user: "ops", port: 2200, group_id: gr.id }),
        conn({ name: "minecraft", host: "mc.home", user: "mc", group_id: sub.id }),
      ];
      const preview: ImportPreview = {
        preview_id: uuid(),
        tool: "OpenSSH",
        groups: [gr, sub],
        connections: items,
        secret_count: 2,
        warnings: ["Host * block: ForwardAgent ignored (agent forwarding is off by default)."],
        duplicates: [{ index: 0, existing_id: existing.id }],
      };
      previews.set(preview.preview_id, preview);
      return structuredClone(preview);
    },
    async importApply(preview_id, skipIndices) {
      await sleep(300);
      const p = previews.get(preview_id);
      if (!p) throw fail("not_found", "Import preview expired — scan again.");
      previews.delete(preview_id);
      db.groups.push(...p.groups);
      const add = p.connections.filter((_, i) => !skipIndices.includes(i));
      db.connections.push(...add);
      return { imported: add.length };
    },
    async syncStatus() {
      return structuredClone(sync);
    },
    async syncGithubStart() {
      githubPolls = 0;
      return { device_code: uuid(), user_code: "WDJB-MJHT", verification_uri: "https://github.com/login/device", interval: 1, expires_in: 900 };
    },
    async syncGithubPoll() {
      githubPolls++;
      if (githubPolls >= 3) {
        sync = { ...sync, config: { kind: "github", owner: "andrea", repo: "handrail-vault", branch: "main" }, last_sync_ms: Date.now() };
        return { done: true, status: "ok", repo: "andrea/handrail-vault" };
      }
      return { done: false, status: "pending" };
    },
    async syncConfigureFolder(path) {
      sync = { ...sync, config: { kind: "folder", path }, last_sync_ms: null, last_report: null, last_error: null };
      syncEvent();
    },
    async syncJoin() {
      if (!sync.config) throw fail("invalid", "sync not configured");
      await sleep(300);
      // Pretend only GitHub and the Dropbox preset already hold a vault.
      if (sync.config.kind === "folder" && !sync.config.path.includes("Dropbox")) {
        throw fail("not_found", "remote has no vault manifest");
      }
      vault.initialized = true;
      vault.locked = true;
    },
    async syncNow() {
      if (!sync.config) throw fail("invalid", "sync not configured");
      sync = { ...sync, syncing: true };
      syncEvent();
      await sleep(500);
      const report = { pulled: 2, pushed: 1, conflicts: 0, warnings: [] };
      sync = { ...sync, syncing: false, last_sync_ms: Date.now(), last_report: report, last_error: null };
      syncEvent();
      return report;
    },
    async syncSetAuto(enabled) {
      sync = { ...sync, auto: enabled };
      syncEvent();
    },
    async syncDisconnect() {
      sync = { ...sync, config: null, last_sync_ms: null, last_report: null, last_error: null };
      syncEvent();
    },
    async syncFolderPresets() {
      const home = "/Users/dev";
      return [
        { drive: "icloud", label: "iCloud Drive", path: `${home}/Library/Mobile Documents/com~apple~CloudDocs/Handrail`, has_vault: false },
        { drive: "dropbox", label: "Dropbox", path: `${home}/Library/CloudStorage/Dropbox/Handrail`, has_vault: true },
      ];
    },

    async vaultSnapshots() {
      return structuredClone([...snapshots].sort((a, b) => b.created_ms - a.created_ms));
    },
    async vaultSnapshotCreate() {
      const created_ms = Date.now();
      const snap = { name: snapName(created_ms), created_ms, size: 4200 };
      snapshots = [...snapshots, snap].sort((a, b) => a.created_ms - b.created_ms).slice(-10);
      return structuredClone(snap);
    },
    async vaultSnapshotRestore(name) {
      if (!snapshots.some((s) => s.name === name)) throw fail("not_found", `snapshot ${name}`);
      await sleep(300);
      await this.vaultSnapshotCreate();
    },
    async vaultConflicts() {
      return structuredClone(conflicts);
    },
    async vaultConflictRestore(fileId, recordedAtMs) {
      if (!conflicts.some((c) => c.file_id === fileId && c.recorded_at_ms === recordedAtMs)) {
        throw fail("not_found", `conflict ${fileId}@${recordedAtMs}`);
      }
    },
    async vaultConflictsClear() {
      conflicts = [];
    },

    async on(event, cb) {
      let set = listeners.get(event);
      if (!set) listeners.set(event, (set = new Set()));
      const fn = cb as (p: unknown) => void;
      set.add(fn);
      const un: Unlisten = () => set!.delete(fn);
      return un;
    },
  };
  return api;
}
