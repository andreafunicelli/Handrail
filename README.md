<p align="center">
  <img src="docs/images/logo.svg" width="96" alt="Handrail Terminal logo" />
</p>

<h1 align="center">Handrail Terminal</h1>

<p align="center">
  <strong>AI in your terminal. You in command.</strong>
</p>

<p align="center">
  An SSH workspace for macOS and Windows that brings your servers, files, and AI agents together.<br />
  Watch agents work in your live terminal, review commands, and take over whenever you need to.
</p>

<p align="center">
  <a href="https://github.com/andreafunicelli/Handrail/releases/latest"><strong>Download</strong></a>
  · <a href="#quick-start">Quick start</a>
  · <a href="#features">Features</a>
  · <a href="#development">Development</a>
  · <a href="https://github.com/andreafunicelli/Handrail/issues">Report an issue</a>
</p>

![Handrail Terminal showing an AI agent checking server disk usage in a live SSH session](docs/images/agent.png)

## Your servers. Your tools. Your call.

Handrail Terminal is for people who manage servers and want AI help with the work in front of them. Organize SSH connections, open remote desktops, move files, and ask an agent to investigate an issue in the same workspace.

The agent uses the terminal you are looking at. Its commands and their output appear in your session, with approvals enforced by the terminal layer. Use the built-in agent with your chosen model provider, or connect Claude Code, Codex, or OpenCode.

## Quick start

1. **Install the app.** Download the latest installer from [GitHub Releases](https://github.com/andreafunicelli/Handrail/releases/latest).

   | Platform | Download |
   | --- | --- |
   | macOS, Apple silicon or Intel | Universal `.dmg` |
   | Windows, x64 | `.exe` installer or `.msi` |

   Current installers and the installed app use the name **Handrail**. Builds are not code-signed yet: on macOS, use **System Settings › Privacy & Security › Open Anyway** after attempting to open the app; on Windows, use **More info › Run anyway** if SmartScreen appears.

2. **Create your vault.** Choose a master password and save the Emergency Kit containing your Secret Key.
3. **Add a connection.** Enter an SSH host or import existing connections from another client.
4. **Connect and work.** Use the terminal and file browser directly. For AI assistance, add a model provider or configure an installed CLI in Settings, then choose an approval mode.

A guided tour runs after vault creation. Replay it from **Settings › About** or **Take the tour** in the command palette. The app also checks for new releases; install an available update with **Update and restart**.

**Setting up another device?** Choose **“I already use Handrail on another device”**, select the same sync destination, and enter the Secret Key from your Emergency Kit and your master password.

## AI assistance with visible commands

Ask the agent to check disk usage, inspect logs, or explain a configuration file. You see the commands it runs and can approve, edit, or deny commands that require review. Press **Esc** to take over.

| Approval mode | What runs automatically | What waits for approval |
| --- | --- | --- |
| **Ask all** | No agent commands | Every command |
| **Ask writes** | Commands classified as reads | Commands classified as writes or destructive |
| **Auto** | Commands classified as reads or writes | Commands classified as destructive |

Commands matching the deny-list always require approval, including in Auto mode. Extend the built-in patterns with plain text or `/regex/` rules. For production connections, the terminal policy treats Auto as Ask writes.

Classification is based on command syntax and known patterns; it does not prove that a command is harmless.

### Choose your agent

| Option | Connect it with |
| --- | --- |
| **Built-in agent, hosted models** | Your API key for Anthropic, OpenAI, DeepSeek, OpenCode Go/Zen, OpenRouter, or a custom OpenAI-compatible endpoint |
| **Built-in agent, local models** | Ollama or LM Studio |
| **External CLI agents** | Your installed Claude Code, Codex, or OpenCode CLI, with its own login, skills, and plugins |
| **OpenChamber** | OpenCode’s web interface in a separate window, selected in an OpenCode profile |

Claude Code and Codex run as their official, unmodified CLIs. You sign in through those tools; Handrail does not read their credential files. Claude Code opens in its own interface beside your SSH tab by default; using the AI sidebar is opt-in.

## Features

### Connections and terminals

- **Organize your servers:** nested groups, search, environment colors, and SSH jump hosts through ProxyJump.
- **Build your workspace:** tabs, split panes, saved workspaces, and a command palette with **Ctrl/⌘ K**.
- **Reuse credentials:** share saved keys and passwords across connections, then rotate them in one place.
- **Save repetitive work:** command snippets with `{{placeholders}}` and suggested read-only diagnostics.
- **Manage tunnels:** saved local (`-L`), remote (`-R`), and SOCKS5 (`-D`) forwarding, restored after reconnect.
- **Watch server health:** CPU, memory, disk, load, and uptime beneath Linux SSH terminals, collected over the existing connection.
- **Work across servers:** broadcast typing or snippets to selected split panes, with receiving panes visibly outlined.

Import connections from `~/.ssh/config`, PuTTY, MobaXterm, mRemoteNG, WinSCP, SecureCRT, and Tabby.

### Files and editing

- **SFTP browser:** browse, create, upload, download, rename, and delete files using the existing SSH login. Switch to root through sudo for system files.
- **Remote code editor:** syntax highlighting, multiple cursors, find and replace, minimap, and indentation guides. Saves check for remote changes and keep a `.bak` of the original.
- **Agent-assisted edits:** ask about a file or selection, then accept or reject the proposed diff.
- **S3-compatible storage:** work with Amazon S3, Cloudflare R2, Backblaze B2, Wasabi, MinIO, and compatible services from the file browser.
- **Copy between destinations:** stream files and folders between servers, or files between a server and S3, without first saving a local copy. Server-to-server transfers preserve permissions; replacements require confirmation.

### Remote desktops

Open Windows desktops over RDP in tabs beside your terminals. Use NLA authentication with vault credentials, keyboard and mouse input, a desktop that resizes with the pane, and an administrative session when needed. Reach hosts behind a bastion through an SSH tunnel using the connection’s jump hosts.

RDP runs in a separate `handrail-rdp` helper process built on [IronRDP](https://github.com/Devolutions/IronRDP).

## Encrypted vault and sync

Connections, saved credentials, and provider keys live in a vault protected by your **master password and Secret Key**, using Argon2id and XChaCha20-Poly1305.

- **Choose your sync destination:** a cloud-drive folder such as iCloud Drive, Dropbox, OneDrive, or Google Drive; a private GitHub repository; or another folder, including Syncthing and network shares.
- **Sync encrypted data:** only ciphertext is sent to the vault’s sync destination. Sync runs after edits and periodically.
- **Recover earlier work:** restore snapshots or recover edits retained during sync conflicts.
- **Start over if needed:** delete the local vault from the unlock screen or **Settings › Vault**. Synced copies are retained.

Keep the Emergency Kit somewhere safe. Vault encryption protects stored and synced data; when you use AI, the selected provider or CLI receives the conversation and tool results needed for that session.

<details>
<summary><strong>More screenshots</strong></summary>

| Connections | Command palette |
| --- | --- |
| ![Connections organized into groups and environments](docs/images/home.png) | ![Command palette for connections and actions](docs/images/palette.png) |

| Agent integrations | Command approvals |
| --- | --- |
| ![Settings for Claude Code, Codex, and OpenCode](docs/images/settings-harnesses.png) | ![Approval modes and deny-list settings](docs/images/settings-approvals.png) |

| Vault sync | Connection import |
| --- | --- |
| ![Cloud-drive, GitHub, and folder sync settings](docs/images/settings-sync.png) | ![Import settings for other SSH clients](docs/images/settings-import.png) |

| Vault setup | Emergency Kit |
| --- | --- |
| ![Creating a vault with a master password](docs/images/onboarding.png) | ![Emergency Kit containing the Secret Key](docs/images/emergency-kit.png) |

</details>

## Development

The app uses **Svelte 5**, **Tauri 2**, and **Rust**. Install Node.js 22, Rust stable, and the [Tauri system prerequisites](https://v2.tauri.app/start/prerequisites/) for your platform, including WebView2 and the C++ build tools on Windows.

```bash
git clone https://github.com/andreafunicelli/Handrail.git
cd Handrail
npm ci
node scripts/build-rdp-helper.mjs
npm run tauri -- dev
```

Build the RDP helper before starting the desktop app; Tauri expects it in `src-tauri/binaries/`.

For UI development without the desktop backend:

```bash
npm run dev
```

This runs the UI in a browser against an in-memory mock backend.

### Checks

After installing dependencies and building the helper:

```bash
npm run check
npm run test
npm run build
cargo test --workspace
cargo test --manifest-path rdp-helper/Cargo.toml --locked
```

The frontend build creates `ui/dist`, which the Tauri build script needs for workspace tests. The RDP helper has its own Cargo workspace and test suite.

For isolated development, set `HANDRAIL_DATA_DIR=<dir>` to use a throwaway vault. `HANDRAIL_FILE_KEYSTORE=1` uses a file for the Secret Key instead of the OS keychain; use it only with test data.

<details>
<summary><strong>Architecture and agent integration</strong></summary>

```text
Svelte UI ── Tauri IPC ── Rust core
                           ├─ aht-term   terminal control and approval policy
                           ├─ aht-agent  built-in agent ──┐
                           └─ aht-mcp    local MCP server ┴─ TerminalTools
                                         ▲
                              Claude Code / Codex / OpenCode
                                      (aht-harness)
```

The built-in agent and external CLIs share `TerminalTools`: `list_connections`, `open_session`, `list_sessions`, `session_info`, `run_command`, `read_output`, and `send_keys`. The terminal layer enforces command approvals. In server mode, the CLI’s own shell and edit tools are disabled so it works through these tools.

The local MCP server listens on `127.0.0.1` with a per-launch bearer token and Host/Origin checks.

| Crate | Responsibility |
| --- | --- |
| `aht-core` | Shared types, `TerminalTools`, and tool specifications |
| `aht-vault` | Encryption, Secret Key, snapshots, and sync merge |
| `aht-ssh` | SSH, known hosts, ProxyJump, agent authentication, and local PTYs |
| `aht-term` | Terminal tracking, shell integration, command execution, and policy |
| `aht-mcp` | Authenticated local MCP server |
| `aht-harness` | Detection, configuration, and launch of external CLI agents |
| `aht-providers` | Anthropic, OpenAI, and OpenAI-compatible streaming adapters |
| `aht-agent` | Built-in agent tool loop |
| `aht-sync` | GitHub, cloud-drive, and folder sync |
| `aht-import` | Connection importers |
| `aht-s3` | S3-compatible storage |

See the [IPC contract](docs/ipc-contract.md) and the [original design specification](docs/superpowers/specs/2026-10-10-aihubterm-design.md), written under the earlier name AIHubTerm.

</details>

## License and credits

See [LICENSE](LICENSE) for the project’s license terms.

Several interface behaviors are adapted from [Interior](https://github.com/ddoemonn/interior) by ozzy (MIT): hold-to-confirm, busy and copy buttons, the sliding choice marker, and parts of the menu, modal, and command palette. Handrail’s Svelte implementations credit the original work in each file.
