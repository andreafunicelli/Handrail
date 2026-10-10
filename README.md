# AIHubTerm

A fast, quiet SSH manager where AI agents work **directly in your live terminal**.

- **Connections in groups** (SSH now; SFTP and MySQL next), with search, nested folders, environment colors, and jump hosts (ProxyJump).
- **Bring your own brain**:
  - API keys for Anthropic, OpenAI, DeepSeek, OpenCode Go/Zen, OpenRouter, Ollama or LM Studio.
  - Your own **Claude Code**, **Codex** or **OpenCode** CLI, using your subscription and your skills and plugins.
- **Agents type into the same terminal you see**, gated by approvals: *Ask all*, *Ask writes* or *Auto*. A deny-list covers destructive commands, and production servers can't silently run in Auto.
- **End-to-end encrypted vault**: Argon2id with a master password plus a Secret Key, and XChaCha20-Poly1305. It syncs through your **private GitHub repo** or any folder (iCloud, Dropbox, Syncthing).
- **Import** from `~/.ssh/config`, PuTTY, MobaXterm, mRemoteNG, WinSCP, SecureCRT and Tabby.

## How agents reach the terminal

```
Svelte UI ── Tauri IPC ── Rust core
                           ├─ aht-term   single control path: shell integration (OSC 133),
                           │             run_command, approval policy, output store
                           ├─ aht-agent  built-in agent ──┐
                           └─ aht-mcp    local MCP server ┴─ same TerminalTools
                                         ▲ 127.0.0.1, per-launch bearer token
                              Claude Code / Codex / OpenCode (aht-harness)
```

Both the built-in agent and external harnesses call the same tools: `list_connections`, `open_session`, `list_sessions`, `session_info`, `run_command`, `read_output` and `send_keys`. Approvals are enforced in `aht-term`, so neither kind of agent can skip them.

**Subscriptions:** Claude Code and Codex always run as their official, unmodified CLIs. You sign in to them yourself, and AIHubTerm never reads their credential files. Claude Code opens in its own interface in a pane next to your SSH tab. Showing it in the AI sidebar instead is an opt-in setting.

## Development

Prerequisites:
- Rust stable
- Node 22
- Platform webview dependencies: WebView2 on Windows; `libwebkit2gtk-4.1-dev` and related packages on Linux

```bash
npm install
```

```bash
npx tauri dev
```

```bash
cargo test --workspace
```

```bash
npm run check && npm run test
```

`npm run dev` alone serves the UI in a browser against an in-memory mock backend.

Set `AIHUBTERM_DATA_DIR=<dir>` to run against a throwaway vault, and `AIHUBTERM_FILE_KEYSTORE=1` to store the Secret Key in a file instead of the OS keychain.

| Crate | Purpose |
|---|---|
| `aht-core` | Shared types, `TerminalTools` trait, tool specs |
| `aht-vault` | Encrypted item store, Secret Key, snapshots, sync merge |
| `aht-ssh` | russh client, known hosts, ProxyJump, agent auth, local PTYs |
| `aht-term` | VT/OSC 133 tracking, hidden shell integration, `run_command`, policy |
| `aht-mcp` | Streamable-HTTP MCP server with bearer auth and Host/Origin checks |
| `aht-harness` | Detect, configure and launch Claude Code / Codex / OpenCode |
| `aht-providers` | Anthropic, OpenAI and OpenAI-compatible streaming adapters |
| `aht-agent` | Built-in tool-loop agent |
| `aht-sync` | GitHub (device flow, Git Data API) and folder sync |
| `aht-import` | Importers for other SSH managers |

Design spec: [`docs/superpowers/specs/2026-10-10-aihubterm-design.md`](docs/superpowers/specs/2026-10-10-aihubterm-design.md)

## Installers and updates

The **Release** workflow (`.github/workflows/release.yml`) builds a universal macOS `.dmg` and Windows `.exe`/`.msi` installers:
- on every push to `main`, published as a GitHub Release `v<major>.<minor>.<build number>`;
- on pull requests, as downloadable workflow artifacts (no release).

The version lives only in the workspace `Cargo.toml`; CI stamps the build number into it. Builds are not code-signed yet (macOS: right-click > Open the first time; Windows: SmartScreen > More info > Run anyway).

Installed apps check for updates on start and every 6 hours and offer "Update and restart"; they never restart on their own. Updates need:
1. An updater key pair: `npx tauri signer generate -w ~/.tauri/aihubterm-updater.key`. The public key goes in `plugins.updater.pubkey` in `src-tauri/tauri.conf.json`; the private key and its password go in the repository secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.
2. A public repository `andreafunicelli/aihubterm-releases` (with a README so `main` exists) and a fine-grained token with "Contents: read and write" on it in the secret `RELEASES_TOKEN`. The app repository is private, and the updater cannot download from a private repository.

## Status

Pre-release (v0.1, milestones M1–M3 in place). Before a public release:
- Choose a license.
- Set up code signing and the updater.
- Confirm with Anthropic and OpenAI that the subscription integration complies with their terms.
