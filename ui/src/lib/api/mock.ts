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

export function seed() {
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
  return { groups, connections, providers, profiles };
}

// ---------- fake shell ----------
const C = {
  user: "\x1b[38;2;124;156;255m",
  path: "\x1b[38;2;139;143;152m",
  dim: "\x1b[2m",
  bold: "\x1b[1m",
  green: "\x1b[38;2;126;201;150m",
  amber: "\x1b[38;2;224;180;110m",
  reset: "\x1b[0m",
};

interface Shell {
  info: SessionInfo;
  local: boolean;
  line: string;
  cwd: string;
  busy: boolean;
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
  const settingsKv = new Map<string, unknown>();
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
    const sh: Shell = { info, local, line: "", cwd: "~", busy: false };
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
    async saveEmergencyKit(contents) {
      // In a real browser a blob download works; the desktop webview needs the backend command.
      if (typeof document !== "undefined" && typeof URL.createObjectURL === "function") {
        const url = URL.createObjectURL(new Blob([contents], { type: "text/plain" }));
        const a = document.createElement("a");
        a.href = url;
        a.download = "AIHubTerm Emergency Kit.txt";
        a.click();
        setTimeout(() => URL.revokeObjectURL(url), 1000);
      }
      return "~/Downloads/AIHubTerm Emergency Kit.txt";
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

    async openSession(connection_id) {
      const c = db.connections.find((x) => x.id === connection_id);
      if (!c) throw fail("not_found", "Connection not found.");
      const mode = c.approval_mode ?? approvalSettings.default_mode;
      const sh = newSession(
        { connection_id, title: c.name, env: c.env, user: c.user, host: c.host, cwd: "~", approval_mode: c.env === "prod" && mode === "auto" ? "confirm_all" : mode },
        false,
      );
      void boot(sh, c);
      return sh.info;
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
    async listSessions() {
      return [...shells.values()].map((s) => s.info);
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
      settingsKv.set("approvals", structuredClone(s));
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
        `│   /help for help · cwd: ~/aihubterm/prof  │\r\n` +
        `╰──────────────────────────────────────────╯${C.reset}\r\n\r\n` +
        `${C.dim}  MCP: aihubterm connected (7 tools)${C.reset}\r\n\r\n> `;
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
        sync = { ...sync, config: { kind: "github", owner: "andrea", repo: "aihubterm-vault", branch: "main" }, last_sync_ms: Date.now() };
        return { done: true, status: "ok", repo: "andrea/aihubterm-vault" };
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
        { drive: "icloud", label: "iCloud Drive", path: `${home}/Library/Mobile Documents/com~apple~CloudDocs/AIHubTerm`, has_vault: false },
        { drive: "dropbox", label: "Dropbox", path: `${home}/Library/CloudStorage/Dropbox/AIHubTerm`, has_vault: true },
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
    async getSetting(key) {
      return (settingsKv.has(key) ? structuredClone(settingsKv.get(key)) : null) as never;
    },
    async setSetting(key, value) {
      settingsKv.set(key, structuredClone(value));
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
