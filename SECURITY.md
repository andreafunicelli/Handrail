# Security policy

Handrail holds server credentials and lets AI agents run commands on real machines, so security
reports are welcome and taken seriously.

## Reporting a vulnerability

Report privately through GitHub: open the repository's **Security** tab and click **Report a
vulnerability**. This creates a private advisory that only you and the maintainer can see.

Please **do not open a public issue, discussion or pull request** for a vulnerability, and do not
include exploit details anywhere public until a fix has been released.

A useful report says which version and platform you used, what an attacker can do, and how to
reproduce it (steps, a proof of concept, or the code path involved).

## Scope

In scope, in particular:

- **The encrypted vault**: key derivation (Argon2id with the master password and Secret Key),
  encryption (XChaCha20-Poly1305), the Secret Key's storage in the OS keychain, snapshots, and
  sync (only ciphertext should ever leave the device).
- **Approval policy bypass**: any way for an agent (built-in or an external CLI) to run a command,
  SQL statement or file edit that the configured approval mode or deny-list should have held for
  the user, including on production connections.
- **The local MCP server** (`aht-mcp`): its `127.0.0.1` listener, per-launch bearer token and
  Host/Origin checks, and anything reachable through it without that token.
- **The updater**: anything that makes an installed app accept an update not signed with the
  release key, or fetch one from somewhere other than this repository's releases.
- **Host-key handling**: SSH host keys accepted without the user's review, or a changed key not
  reported as a mismatch (including through jump hosts and "Test connection").

Also in scope: secrets leaking into logs, IPC responses, the operations history or synced files,
and the boundary between the webview and the Rust core (IPC commands reachable with unexpected
arguments).

Out of scope: problems in third-party CLIs (Claude Code, Codex, OpenCode) or model providers
themselves, attacks that need an already compromised user account on the same computer, and the
lack of code signing described below.

## Supported versions

Only the **latest stable release** receives security fixes. Nightly prereleases are for testing
and are replaced by the next build.

## Code signing

Release builds are **not code-signed yet**: macOS shows a Gatekeeper warning and Windows a
SmartScreen warning on first launch. Download installers only from this repository's
[releases page](https://github.com/andreafunicelli/Handrail/releases). Once installed, updates
are verified against the updater's signing key before they are applied.
