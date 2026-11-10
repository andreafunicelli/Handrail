# Handrail 1.1 — UI/UX redesign: plan and checklist

Branch: `feature/handrail-v1.1-ux` (kept out of `main`: every push to `main` publishes a
release). The first release containing this work is **v1.1.0**; later fixes are 1.1.x
(`release.yml` takes `major.minor` from the workspace version and the patch from the tags).

This document is updated as the work lands. `[x]` = implemented and checked, `[~]` = partly,
`[ ]` = not done (with the reason when it will not be done in 1.1).

## Audit summary (what existed before 1.1)

- **Preferences**: per-device in localStorage (`aht:prefs`, `pinned`, `recent`, `workspaces`,
  `collapsed`, `conn-order`, `lang`). Only approval settings live in the vault (`approvals`
  setting). The vault offers a generic encrypted, synced key → JSON store (`get_setting` /
  `set_setting`).
- **Connections**: env (prod/staging/dev/none), tags, notes, jump hosts, per-connection approval
  mode, identities. Pinned and recent were shown only on the start page. Search was fuzzy text.
- **Sessions/workspaces**: tabs of split layouts whose leaves are typed strings (terminal id,
  `view:` for RDP/DB, `files:`, `s3:`, `edit:`, `backup:`, `pending:`). Workspaces store layouts
  of connection ids (localStorage), restored by opening fresh logins.
- **Terminal context**: tab env stripe, pane headers only when split, SessionBar polls CPU, memory,
  disk, load, uptime and tunnels every 5 s over its own SSH channel.
- **AI**: one event stream for the built-in agent and the harnesses in sidebar mode
  (text, thinking, tool_call, tool_result with `CommandResult` = state, exit code, duration,
  truncated, output id; approval requested/resolved; usage; error). Harnesses in native or
  OpenChamber mode run in their own window: no events. Transcripts in memory only.
- **Approvals**: enforced in Rust (`aht-term` policy: modes, deny list, prod downgrade of Auto,
  risk classifier); the UI only answers (`approve`, `approve_edited`, `deny`). Edited commands are
  re-evaluated by the backend. Hold-to-confirm for destructive commands.
- **No audit/history persistence** anywhere.
- **Files**: SFTP and S3 backends, cross-host copy (SFTP↔SFTP, S3↔SFTP), overwrite prompt,
  transfers store without cancel. **No local filesystem backend.**
- **Connection editor**: one long form; **no connection test**.
- **i18n**: `t()` over an English catalog (~1100 keys), almost no hardcoded strings.
- **Tests**: vitest (unit), Rust unit + integration tests (real sshd, MySQL, PostgreSQL, S3 in CI).
  No browser e2e suite (screens were checked with Playwright scripts against the mock API).

## Classification and checklist

Legend: UX = existing feature, UX change; EXT = existing feature extended; FE = new frontend;
BE = new backend; X = cross-cutting.

### Phase 1 — Foundation
- [x] X — Type scale, control heights, spacing, focus and production tokens (`app.css`);
  components use the scale instead of fixed pixel sizes.
- [x] FE — Interface density (Comfortable default, Compact) in Settings › Appearance; terminal
  and editor font sizes unaffected.
- [x] X — Reactive language with English fallback; Italian catalog; language selector; tests.
- [x] X — Version 1.1.0 (workspace, crates, npm package).
- [ ] UI scale preference — not added: density covers text and control size, and a CSS zoom
  would also scale the terminal, which must keep its own size.

### Phase 2 — Navigation and workspaces
- [x] EXT — Sidebar filter syntax (`env:`, `type:`, `tag:`, `host:`, `user:`, `group:`,
  `is:active`, `is:fav`, `-` to exclude, lists), chips, filter menu; filters keep the folder tree.
- [x] FE — Active sessions section (focuses the open pane; numbered duplicates).
- [x] UX — Favorites and Recent sections in the sidebar (same data as before), collapsible.
- [x] UX — Connect / more buttons on rows; connect focuses an open session.
- [x] UX — Home: Resume last workspace (with a summary of what it holds), other workspaces,
  favorites, recent connections, quick actions, first-run paths. Restoring skips panes whose
  connection was deleted (it used to open a local shell in their place).
- [x] UX — Command palette sections (open sessions, favorites, connections, workspaces, AI, files,
  databases, settings), shortcut hints, active-session context, more actions.

### Phase 3 — Terminal
- [x] UX — Session context in the bar under SSH terminals (user@host, via jump host, status,
  agent); the name stays in the tab and pane header.
- [x] UX — PROD label in tabs (a tab takes its most sensitive pane), split pane headers, session
  bar, sidebar, palette, home, AI panel, approval cards and the database toolbar.
- [x] UX — Metrics: labels, tooltips with units, last refresh, stale/unavailable/disconnected
  states (no old values shown as live), warn ≥ 80 % and danger ≥ 90 %.
- [x] FE — Quick actions (SFTP, new terminal, split, copy host/address, DB via host, details)
  from the bar, the terminal context menu and the tab menu.

### Phase 4 — AI
- [x] FE — Summary / Activity / Chat views (activity from real events; summary from real
  metadata, model summary only on request). Harnesses in native/OpenChamber mode: a note, no
  invented events.
- [x] UX — Agent status states; approval queue always visible (tab badge, banner in every view,
  pending cards pinned in Activity, approvals waiting in other sessions listed). Requests the
  backend withdraws unanswered (`approval://resolved`) show as expired.
- [x] UX — Approval card context (host, env, user, agent, risk, reason, policy, technical
  details; edited commands are re-checked by the backend policy, as the card says).
- [~] FE — Operations history (see Decisions): vault setting `opslog.<device id>`, settings
  `opslog.settings`, redaction, retention, cap, clear, filters in Settings › Approvals. Covers
  agent tool calls seen by the UI; not database query approvals, nor harnesses running in their
  own window (their calls never reach this UI). No backend change: written by the UI in batches.

### Phase 5 — Operational tools
- [x] UX — Database: focus SQL mode, resizable editor/results split (kept per device),
  collapsible schema, open database and PROD in the toolbar, titled errors with copy.
- [x] FE — Dual-pane file manager: a second pane on any connected SSH session or S3 account;
  copy between panes (F5) where `copyBetween` supports it (S3 → S3 and folders to/from S3 are
  explained, not offered); move (F6) only inside one file system (a rename); Tab switches pane.
  No local pane (no backend local file access): Upload/Download, and the bar says so. Large
  folders render rows in steps of 300 (5000 rows: ~1.2 s → ~0.12 s to first paint, mock).
- [x] FE — Global transfer panel on the icon rail: running badge; source, destination, bytes,
  percent, done/failed with the error; in memory only; no cancel (the backend cannot stop one).
- [x] FE — Compare before overwrite: copies (backend `exists`) and uploads (destination listed
  first, since uploads replace silently) open a comparison of path, size, time, type and
  permissions, with an on-demand line diff for text files up to 256 KB on both sides; local
  files, folders, binary or bigger files get an explicit "no line comparison" reason.
- [x] UX — Connection editor sections (General, Authentication, Network, Advanced; the last two
  open themselves when they hold values or an error).
- [x] BE — Test connection without saving: SSH login through jump hosts (no shell, no command;
  unknown host keys are refused and reported with their fingerprint, never recorded), database
  connect/close through the tunnel, RDP reachability only ("credentials not checked"); 15 s.

### Phase 6–7 — Polish and release
- [x] Onboarding: a new vault's home screen offers import, add by hand, or set up AI (built-in
  agent or external CLI), with a privacy note; the tour still runs and can be replayed.
- [x] Italian for every key (1480), checked by a test (keys and placeholders).
- [~] Accessibility, responsive and performance review: see the release readiness report.
- [x] README, CHANGELOG, release notes (`docs/release-notes-1.1.0.md`).
- [ ] Release readiness: desktop validation on macOS, Windows and Linux (see the report).

## Decisions

- **Workspaces stay per device** (localStorage) as before; no credentials are stored in them.
- **Operations history** is kept encrypted in the vault (generic setting store), one record list
  per device so two devices never overwrite each other. It stores metadata (time, session,
  connection, agent, decision, outcome, exit code, duration) and the command with known secret
  patterns redacted; never terminal output. Retention and clearing are configurable; a locked
  vault cannot be read.
- **No local filesystem pane**: the backend has no local file access API; the dual pane offers
  SSH and S3 destinations only, and says so.
