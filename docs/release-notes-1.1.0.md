# Handrail 1.1.0 — Workspace Experience Update

Handrail 1.1.0 reworks the interface around the session in front of you: where you are, what the
agent is doing, and what a file operation is about to replace. The complete list is in the
[changelog](../CHANGELOG.md).

## Highlights

- **Home** resumes your last workspace and lists favorites, recent connections and quick actions.
  On a new vault it offers three first steps: import connections, add one by hand, or set up AI.
- **Sidebar filters** (`env:prod`, `type:ssh`, `tag:web`, `is:active`, `-env:dev`) with chips,
  and sections for active sessions, favorites and recent connections.
- **Production is labelled** with the word PROD, not only a colour, wherever a session appears.
- **Terminal bar**: user, host, jump host, session status, metrics marked stale when they are,
  and quick actions.
- **AI sidebar**: Summary, Activity and Chat views; approvals that stay visible in every view; an
  encrypted operations history in Settings › Approvals.
- **Files**: dual pane, with this computer as one side (F5 copy, F6 move, Tab to switch), a
  transfer panel, and a comparison before an existing file is replaced.
- **Connection editor** in sections, with **Test connection** before saving.
- **Databases**: focus mode, a resizable editor, export and import, and an SQL backup viewer.
- **Interface density** (Comfortable, Compact) and an **Italian** translation.

## Compatibility

- **Existing vaults open unchanged.** 1.1 adds only optional data: the operations history and its
  settings (`opslog.*` vault settings) are created the first time they are used. A device still
  on 1.0 ignores them.
- **Preferences** stored on the device (layout, sidebar sections, density, language) start from
  their defaults; nothing from 1.0 is lost. Saved workspaces created with 1.0 open as before.
- **Approval rules are unchanged.** The terminal layer still decides which commands need
  approval; 1.1 changes only how requests are shown.
- Updating from 1.0.x uses the built-in updater, as for earlier releases.

## Known limitations

- **Folders between this computer and a server or S3** are not copied yet: copy the files, or
  copy folders between servers.
- **Transfers cannot be cancelled** from the transfer panel.
- **S3 to S3 copies, and folders to or from S3**, are not offered between panes; the reason is
  shown.
- **RDP Test connection** checks only that the server answers. It does not check credentials.
- **Operations history** is per device and covers agent tool calls seen by the app. It does not
  cover database query approvals or external CLIs running in their own window. Masking of
  secrets in commands relies on known patterns; turn off "record command text" to store none.
- **Activity and Summary** are unavailable for external CLIs running in their own window or in
  OpenChamber, because their steps never reach Handrail. The panel says so.
- **Installers are not code-signed.** See the [quick start](../README.md#quick-start) for opening
  the app on macOS and Windows.
- **Automatic updates** need release artifacts signed with the updater key. If a release is
  published without them, download the installer from the release page.
