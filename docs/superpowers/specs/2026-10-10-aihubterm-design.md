# AIHubTerm — Design Spec

Date: 2026-10-10 · Status: approved in brainstorming, building v1

## 1. Purpose

AIHubTerm is a sleek, lightweight, cross-platform connection manager (SSH first; MySQL, SFTP later)
where AI agents work **directly in the live terminal** of the server you are connected to.

Product goals (released to other developers; business model undecided, so local-first with a
pluggable sync layer):

- **AI-native:** providers and agent harnesses are configured once and are instantly available on
  every connection. The agent types commands into the same live terminal the user sees.
- **Bring your own brain:** API-key providers (Anthropic, OpenAI, DeepSeek, OpenCode Go/Zen,
  OpenRouter, Ollama/LM Studio) **and** subscription-backed harnesses via their official CLIs
  (Claude Code for Claude subscriptions, Codex CLI for ChatGPT, OpenCode CLI). Harness skills and
  plugins selected by the user load natively because the real CLI is run.
- **Sleek & light:** quiet dark-first UI, cold start < 1 s, minimal idle RAM.
- **Platforms:** Windows, macOS, Linux in v1. iOS/Android in v2 (API agent only — mobile cannot
  spawn harness CLIs).

Non-goals for v1: MySQL client (v1.2, own spec, Tabularis/HeidiSQL-class), SFTP (v1.1), RDP/VNC,
containers, monitoring, hosted backend, teams.

## 2. Stack

- **Tauri 2** desktop shell, **Rust** core (Cargo workspace), **Svelte 5 + Vite + TypeScript** UI.
- Terminal rendering: **xterm.js** (`@xterm/xterm`, WebGL + fit addons).
- SSH: `russh`. Local shells: `portable-pty`. MCP: `rmcp` (official Rust SDK, streamable HTTP).
- Crypto: `argon2` (Argon2id), `chacha20poly1305` (XChaCha20-Poly1305), `zeroize`, OS keychain via
  `keyring`.

## 3. Architecture

```
Svelte 5 UI (webview)  ── Tauri IPC (commands + events) ──  Rust core
```

Cargo workspace, one crate per responsibility so each unit is testable alone:

| Crate | Responsibility | Depends on |
|---|---|---|
| `aht-core` | Shared types: ids, `Connection`, `Group`, `Provider`, `HarnessProfile`, errors | — |
| `aht-vault` | Encrypted item store, master password + Secret Key, lock/unlock, snapshots | core |
| `aht-ssh` | SSH sessions (russh), host-key verification, ProxyJump, agent/Pageant, local PTY | core |
| `aht-term` | Session byte stream fan-out, shell-integration injection, OSC 133 parser, command runner, output store, **policy/approval engine** | core |
| `aht-mcp` | Local MCP server exposing terminal tools; token auth | core, term (via trait) |
| `aht-harness` | Detect/launch/supervise Claude Code, Codex, OpenCode; profiles; stream adapters | core |
| `aht-providers` | LLM adapters: Anthropic Messages, OpenAI Chat Completions, OpenAI-compatible | core |
| `aht-agent` | Built-in agent tool loop over `aht-providers` + the terminal tool set | providers, term |
| `aht-sync` | `SyncBackend` trait, GitHub (REST, device flow) and local-folder backends, merge | core, vault |
| `aht-import` | Importers: ssh_config, PuTTY, MobaXterm, mRemoteNG, WinSCP, SecureCRT, Tabby | core |
| `src-tauri` | App glue: state, IPC commands/events, wiring crates together | all |

**Single control path into a terminal.** Everything that acts on a terminal — the built-in agent,
harnesses over MCP, UI snippets — goes through `aht-term`'s `TerminalControl` API. Approval policy
is enforced there and cannot be bypassed. Credentials never leave the vault/ssh crates.

## 4. Data model (`aht-core`)

- `Connection { id, name, kind: Ssh|Mysql|Sftp (only Ssh active in v1), group_id, host, port, user,
  auth: Password(secret_ref)|Key(key_ref, passphrase_ref?)|Agent, jump: Vec<ConnectionId>,
  env: Prod|Staging|Dev|None, tags, color?, approval_mode_override?, notes }`
- `Group { id, name, parent_id?, order, kind_filter? }` — user folders, nested; UI also shows
  type tabs (SSH / MySQL / SFTP).
- `Secret` items (passwords, private keys, API keys) are separate vault items referenced by id.
- `Provider { id, name, kind: Anthropic|OpenAI|OpenAICompat, base_url, api_key_ref, default_model,
  preset: DeepSeek|OpenCodeGo|OpenCodeZen|OpenRouter|Ollama|LmStudio|Custom }`
- `HarnessProfile { id, name, harness: ClaudeCode|Codex|OpenCode, display: Sidebar|Native,
  server_mode: bool (default true), skills: Vec<path>, plugins: Vec<path>, extra_args }`
- `ApprovalMode`: `ConfirmAll | ConfirmWrites | Auto` (+ deny-list always confirms).
  Default `ConfirmAll`; `Prod` connections default `ConfirmAll` and cannot be `Auto` without an
  explicit typed confirmation.

## 5. Vault (`aht-vault`)

- KDF: Argon2id(master password, salt) → combined with 128-bit **Secret Key** via HKDF-SHA256 →
  vault key. XChaCha20-Poly1305 per item, random 24-byte nonce, item id as AAD.
- On disk: app data dir `vault/manifest.enc` (format version, KDF params, key check) +
  `vault/items/<random-id>.enc` + `vault/tombstones/`. Same layout is what sync uploads.
- Each item carries `{id, type, updated_at (HLC timestamp), device_id, payload}` inside the
  ciphertext.
- Unlock caching: optional OS keychain storage of the vault key for biometric unlock (Windows Hello
  / Touch ID via OS keychain prompts) — M4. Auto-lock after inactivity (default 15 min).
- Secret Key shown once at setup in an "Emergency Kit" (text + QR); new devices need it.
- Local snapshots: last 10 encrypted snapshots of the item set for rollback.
- Keys and plaintexts are `Zeroizing`.

## 6. SSH (`aht-ssh`)

- russh client; auth via password, private key (ed25519, ECDSA, RSA; OpenSSH format, passphrase),
  ssh-agent (Unix socket / Windows OpenSSH agent pipe / Pageant).
- Host keys: app-managed known_hosts (also reads `~/.ssh/known_hosts`); TOFU prompt on first
  connect; **changed key = blocking full-screen warning**, connect refused unless user explicitly
  replaces.
- ProxyJump chains via `direct-tcpip` channels.
- PTY request with resize events; agent forwarding off by default.
- Local shells (PowerShell, cmd, Git Bash, WSL, bash/zsh) via portable-pty using the same session
  interface.
- Reconnect with exponential backoff on drop; session marked `Reconnecting`.

## 7. Terminal control (`aht-term`)

**Session stream.** Each tab owns a `Session` producing bytes. `aht-term` fans out to (a) the UI
(xterm.js via Tauri events, base64 chunks) and (b) a parser that tracks OSC 133 marks, alt-screen
state (`CSI ?1049h/l`) and a ring buffer of recent output.

**Shell integration.** After login the core detects the shell (`echo $0`-style probe with hidden
output) and injects a one-line hook for bash/zsh/fish emitting OSC 133 `A` (prompt start), `B`
(prompt end), `C` (command start), `D;<exit>` (command end). Injection output is suppressed from
the UI by filtering between private sentinel markers. Nothing is written to the server's disk.

**Fallback ("limited AI mode").** If no OSC 133 within a timeout: learn the prompt regex from the
last line before input and use output-quiescence (default 1.5 s) to end commands; exit codes
reported as `unknown`. UI shows a "limited AI mode" badge.

**Command execution** — `run_command(session_id, command, timeout_s?)`:
1. Policy check (approval mode, per-connection override, deny-list regexes, risk class
   read/write/destructive).
2. If approval needed → emit `approval_requested` event → UI shows inline card → wait for decision
   (approve / edit-and-approve / deny).
3. Wait until the shell is at a prompt (state `Idle` from OSC 133 `A/B`). Refuse if alt-screen or
   a foreground program is running.
4. Write command + `\r` to the PTY; mark the line as agent-run (UI decoration ✦).
5. Collect output between `C` and `D`; strip ANSI; store full output under `output_id`.
6. Return `{exit_code?, output (head 4 KB + tail 4 KB when truncated), truncated, output_id,
   duration_ms, state: Completed|TimedOut|Interrupted|NeedsUser}`.

**Pause / hand-back.** Agent pauses and returns `NeedsUser` when: alt-screen enters, output
matches password-prompt patterns (`[Pp]assword.*:`, `passphrase`, `[sudo]`), or the user types
while an agent command runs. The agent never sees or types user secrets.

**Concurrency.** One agent per session; a per-session async lock. While held, UI shows "Agent
running — Esc to take over"; user keyboard input pauses the agent.

**Tools (shared by built-in agent and MCP):** `list_connections`, `open_session(connection_id)`,
`list_sessions`, `session_info(session_id)` (os, shell, cwd, user, integration mode),
`run_command`, `read_output(output_id, offset, limit)`, `send_keys(session_id, keys)` — only
control keys (`ctrl-c`, `ctrl-d`, `enter`, `q`, `esc`, arrows).

**Audit log.** Every agent action appended per session (who: brain id; command; decision; exit).

## 8. MCP server (`aht-mcp`)

- rmcp streamable-HTTP server bound to `127.0.0.1:<random>` started lazily with the first harness.
- Bearer token generated per harness launch; requests without a valid token rejected. Tokens are
  bound to a harness process; revoked when it exits. Each token maps to an allowed session scope
  (default: the tab that launched it + `open_session` on any saved connection).
- Exposes exactly the tools in §7 via a `TerminalTools` trait implemented in `src-tauri` on top of
  `aht-term`, so the server is testable with a mock.
- Never returns credentials or vault data.

## 9. Harnesses (`aht-harness`)

- Detection: locate `claude`, `codex`, `opencode` on PATH and common install dirs; report version
  and login status.
- Sign-in: opens a local terminal tab running the CLI's own login command.
- Launch (per profile, per tab): working dir = per-profile dir under app data with selected
  skills/plugins linked in; MCP wiring via launch flags only (Claude Code `--mcp-config` +
  `--strict-mcp-config`; Codex `-c mcp_servers...`; OpenCode via env-pointed config). Session
  context appended to the system prompt. In server mode, local shell/edit tools are disabled and
  `mcp__aihubterm__*` tools pre-allowed — approvals happen in our policy layer.
- Display modes: **Sidebar** (headless streaming JSON → normalized `AgentEvent`s rendered like the
  built-in agent) and **Native** (CLI TUI in a local PTY pane beside the SSH tab, still MCP-wired).
- Subscription compliance (see `docs/research/2026-10-10-integration-research.md`): Anthropic
  forbids third-party apps from offering Claude.ai login or routing requests through subscription
  credentials, while allowing an end user to run the unmodified Claude Code binary with their own
  subscription. Therefore: Claude Code defaults to **Native** display (the user's own CLI in a PTY);
  headless Sidebar mode is an opt-in "advanced" setting with a disclaimer. We never read harness
  credential files, never inject OAuth tokens, and scrub `ANTHROPIC_API_KEY` from the child env
  unless the profile explicitly opts into API billing. Same rules for Codex/ChatGPT. Obtain written
  confirmation from Anthropic/OpenAI before commercial release.
- Supervision: restart on crash; chat transcript persisted; SSH session unaffected.
- Later harnesses (v1.3): Hermes, others — added as config + stream adapter.

## 10. Providers & built-in agent

- Adapters: Anthropic Messages (streaming, tool use), OpenAI Chat Completions (streaming, tools),
  OpenAI-compatible (base URL + key; presets for DeepSeek, OpenCode Go, OpenCode Zen, OpenRouter,
  Ollama, LM Studio).
- Model list fetched from `/models` where supported; manual entry otherwise.
- Built-in agent: system prompt with session context, tool loop over the §7 tools, max-steps
  guard (default 25), streaming events: `text_delta`, `tool_call`, `tool_result`,
  `approval_requested`, `error`, `done`; token & cost counters.
- Provider errors (401/429/5xx/model missing) surface as sidebar cards with "switch brain".

## 11. Sync (`aht-sync`)

- `SyncBackend` trait: `list() -> [(path, etag)]`, `get(path)`, `put(path, bytes, expected_etag?)`,
  `delete(path)`.
- Backends v1: **GitHub** (REST contents/git-data API, private repo, OAuth **device flow**, token in
  OS keychain) and **Local folder** (iCloud Drive/Dropbox/Syncthing).
- Only ciphertext and random file names are uploaded. Merge per item by HLC timestamp
  (last-writer-wins); losing version kept in local history; tombstones propagate deletions.
- Synced: connections, groups, secrets, providers, approval settings, snippets, harness profiles
  (minus machine paths), known_hosts trust decisions. Not synced: harness logins.

## 12. Importers (`aht-import`)

Common interface `Importer { detect() -> Vec<Source>, parse(source) -> ImportBundle }` producing
connections, groups (preserving source folders) and optional secrets. Preview screen with
duplicate detection (host+port+user). v1: `~/.ssh/config`, PuTTY (registry), MobaXterm
(`.mxtsessions`, `MobaXterm.ini`), mRemoteNG (`confCons.xml`, default-key password decryption),
WinSCP (ini/registry, password de-obfuscation), SecureCRT (XML export), Tabby (`config.yaml`).

## 13. UI (Svelte 5)

- Quiet-instrument aesthetic: near-black neutrals, one accent, 1 px dividers, no gradients/glow;
  light theme; fonts Geist (UI) + JetBrains Mono (terminal), bundled. Terminal schemes importable
  (iTerm2 `.itermcolors`, Windows Terminal JSON).
- Layout: icon rail · connection sidebar (search, type tabs, nested folders, status dots, env
  colors, drag & drop, quick-connect) · tab strip with splits · collapsible AI sidebar (⌘/Ctrl+J)
  with brain picker, approval mode, chat, tool cards linked to terminal markers.
- Command palette ⌘/Ctrl+K. Start page with recent/pinned. Saved workspaces.
- Responsive from day one; all strings through an i18n layer (EN first).
- Performance budgets: cold start < 1 s; idle RAM measured per OS each release; xterm WebGL
  renderer; virtualized lists; no idle timers.

## 14. Security summary

Vault crypto (§5); host-key TOFU with hard changed-key block; MCP localhost + per-launch tokens;
no credentials in tool outputs; server output treated as untrusted (prompt injection) — approval
modes + deny-list are the guard; Prod cannot be `Auto` silently; no telemetry by default; signed
builds, macOS notarization, signed updater; `cargo-deny`/`cargo-audit` in CI.

## 15. Error handling

SSH drop → reconnect with backoff, running agent command marked `Interrupted`, never replayed.
Provider errors → sidebar card. Harness crash → restart, session survives. Vault corruption →
snapshot rollback. Sync conflict → LWW + history + notice.

## 16. Testing

- Unit: vault crypto vectors & tamper detection, policy engine, OSC 133 parser, prompt fallback,
  importers against fixture files, provider adapters against recorded responses.
- Integration: Docker sshd containers (bash, zsh, fish, busybox, alpine) for shell integration and
  fallback; MCP server with an rmcp client; fake LLM provider for deterministic agent loops.
- E2E: `tauri-driver` on Windows/macOS/Linux CI matrix; startup-time + RAM benchmark job.

## 17. Roadmap

- **M1** app shell, vault, SSH tabs/splits, groups, importers
- **M2** terminal control layer, MCP server, Claude Code / Codex / OpenCode harnesses, approvals
- **M3** built-in agent + providers
- **M4** sync (GitHub, folder), biometric unlock
- **M5** polish, signing, auto-update → v1.0
- v1.1 SFTP, port forwarding UI · v1.2 MySQL client (own spec) · v1.3 Hermes + more harnesses,
  PostgreSQL · v2 iOS/Android, Docker/K8s, monitoring

## 18. Open decisions

- License (Apache-2.0 vs AGPL-3.0) — must be chosen before the repo goes public; does not block
  development.
