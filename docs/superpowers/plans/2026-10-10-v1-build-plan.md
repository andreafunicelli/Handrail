# AIHubTerm v1 — Build Plan

Spec: `docs/superpowers/specs/2026-10-10-aihubterm-design.md` · IPC: `docs/ipc-contract.md`

## Phase A — parallel crate work (one worktree + branch each, under `../AIHubTerm-wt/`)

| Branch | Scope | Model |
|---|---|---|
| `feat/vault` | `aht-vault`: Argon2id + Secret Key + XChaCha20, item store, HLC, tombstones, merge, snapshots | Opus |
| `feat/ssh` | `aht-ssh`: russh client, host keys, ProxyJump, agent auth, local PTY | Opus |
| `feat/term` | `aht-term`: VT/OSC 133 parser, shell-integration injection, run_command, policy engine, output store | Opus |
| `feat/mcp` | `aht-mcp`: rmcp streamable-HTTP server, bearer auth, Host/Origin checks | Opus |
| `feat/harness` | `aht-harness`: detection, launch specs, stream adapters, headless runner | Sonnet |
| `feat/agent` | `aht-providers` + `aht-agent`: Anthropic/OpenAI/compat adapters, tool loop | Sonnet |
| `feat/import` | `aht-import`: ssh_config, PuTTY, MobaXterm, mRemoteNG, WinSCP, SecureCRT, Tabby | Sonnet |
| `feat/ui` | Svelte 5 UI against the IPC contract with a mock backend | Opus |
| `feat/ci` | GitHub Actions matrix, cargo-deny, toolchain | Haiku |

Each branch: crate-local tests + clippy clean, committed. Lead reviews and merges into `main`.

## Phase B — integration (after merges)

1. `aht-sync`: SyncBackend trait, local-folder + GitHub (device flow, REST) backends over vault raw files (Sonnet).
2. `src-tauri`: AppState wiring vault ⇄ ssh ⇄ term ⇄ mcp ⇄ harness ⇄ agent; implement every IPC command/event; `TerminalTools` impl over aht-term sessions; Approver bridging to UI events (Opus/lead).
3. End-to-end smoke: `tauri dev`, local shell tab, agent run with mock provider, MCP check with a harness in Native mode.
4. Review pass (code-review skill) and fix-up.

## Phase C — hardening toward v1.0 (M4/M5)

Biometric unlock, auto-lock, signing/notarization, updater, RAM/startup benchmarks, tauri-driver E2E.
