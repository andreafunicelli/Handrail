# Changelog

Release notes for each version are published with its
[GitHub Release](https://github.com/andreafunicelli/Handrail/releases). Releases before 1.1.0
list the pull requests merged since the previous release.

## Handrail 1.1.0 — Workspace Experience Update

The first release of the 1.1 series. Later fixes are released as 1.1.x.

### UI/UX
- New type scale and control sizes. **Interface density** (Settings › Appearance): Comfortable,
  the default, with 14 px text, or Compact for more on screen. The terminal and the code editor
  keep their own font sizes.
- Consistent design tokens for text sizes, control heights, spacing, focus and production
  context.
- **Home screen**:
  - resume the last workspace, with a summary of what it holds;
  - the other saved workspaces, favorites and recent connections;
  - quick actions: new SSH or database connection, local shell, open a workspace, import,
    configure AI.

### Navigation
- **Connection search filters**, combined with fuzzy text:
  - `env:prod`, `type:ssh|rdp|mysql|postgres|db`, `tag:`, `host:`, `user:`, `group:`;
  - `is:active`, `is:fav`;
  - `-` excludes a match, and `env:prod,staging` gives a list.
  - Active filters show as removable chips, and a filter menu adds the common ones.
  - Filters alone keep the folder tree.
- **Active sessions** section in the sidebar: every open session, numbered when a connection is
  open twice. Clicking one focuses it.
- **Favorites and Recent** sections in the sidebar, collapsible.
- Connection rows have connect and actions buttons. Connecting to a connection that is already
  open goes to its session.
- **Command palette**:
  - results grouped into open sessions, favorites, connections, workspaces, AI, files,
    databases and settings;
  - keyboard shortcuts shown next to actions;
  - the active session at the top.
- Restoring a workspace skips panes whose connection was deleted, with a notice. It used to open
  a local shell in their place.

### Terminal
- An explicit **PROD** label on production sessions, in addition to the colour, in:
  - tabs: a tab is marked when any of its panes is production;
  - split pane headers and the bar under the terminal;
  - the sidebar, the palette, the AI panel and approval cards.
- **The bar under SSH terminals** shows:
  - the user and host, the jump host used and the session status;
  - CPU, memory and disk as percentages, with tooltips, units and the time of the last reading;
  - warning colours at 80 % and danger colours at 90 %;
  - explicit stale, unavailable and disconnected states, so old values are never shown as live.
- **Quick actions** from the bar, the terminal's context menu and the tab menu:
  - open SFTP, new terminal, split right;
  - copy host name, copy address;
  - open a database through this host;
  - connection details.

### AI
- **Three views** in the AI sidebar: **Summary**, **Activity** and **Chat**. Switching views never
  interrupts a running agent.
- **Activity** is a timeline of what the agent did, built from real events only:
  - each step shows its command, target, risk, approval decision, status, exit code, duration
    and output;
  - each step links back to the terminal;
  - steps can be filtered by status and tool, and pending approvals are pinned at the top.
- **Summary** is assembled from the same events, with no model call:
  - the request and the target;
  - the commands run and the errors;
  - the changes made;
  - the agent's latest message, quoted word for word.
  - A button can ask the agent for a summary as a normal chat message (this uses tokens).
- **Agent status**: idle, thinking, running command, waiting for approval, completed, failed,
  cancelled.
- External CLIs running in their own window show a note instead of invented activity.

### Security and approvals
- **Approval cards** show the full command, the target connection and host, environment,
  effective user, requesting agent, risk, the agent's reason and the policy note. Technical
  details expand on demand.
- An edited command is still checked again by the terminal policy, and the card says so.
- Pending approvals cannot be missed: a badge on Activity, and a banner in every view that also
  lists approvals waiting in other sessions.
- Requests the backend withdraws unanswered now show as expired. They used to stay pending.
- **Operations history**:
  - Settings › Approvals lists agent operations, kept encrypted in the vault, one list per
    device.
  - Each entry: time, connection, agent, tool, decision, outcome, exit code, duration, and the
    command with known secret patterns masked.
  - Terminal output is never stored.
  - Retention, a size cap, "record command text" and Clear history are configurable.
  - Nothing can be read while the vault is locked.
- No approval rule changed. The terminal layer still decides; the interface only displays and
  forwards your decision.

### Database
- **Focus SQL** mode.
- A resizable splitter between editor and results, remembered on each device.
- Collapsible schema panel.
- The open database (with PROD) named in the toolbar.
- Clearer query errors, with a copy button.
- **SQL backup viewer**:
  - open a `.sql` or `.sql.gz` file (`mysqldump`, `pg_dump` plain format, Handrail's own export)
    without a server;
  - browse its tables and rows, with a filter;
  - save a table as CSV, JSON or SQL ready to restore.
- **Export and import**:
  - export a database or schema to SQL;
  - import SQL files, including `pg_dump`'s `COPY` blocks;
  - import CSV into a table.
- Account management from the server panel.

### File management
- **Dual-pane file manager**: a second pane on **this computer**, any connected SSH server, or
  an S3 account.
  - Copy between the panes with F5, where supported: files both ways between this computer and
    a server or S3; files and folders between servers.
  - Move within one file system with F6.
  - Switch panes with Tab.
  - On this computer: browse, create folders, rename and delete (after a confirmation). Windows
    drives are listed at the top level.
- **Transfer panel** on the icon rail: uploads, downloads and copies, with progress, completion
  and errors. A failed transfer is never shown as done.
- **Compare before overwrite** for copies and uploads:
  - shows the path, size, time, type and permissions of both files;
  - offers a line diff for text files up to 256 KB.
- Large folders render progressively.

### Settings
- Interface density and language (English, Italian) in Settings › Appearance.
- **Connection editor** in sections: General, Authentication, Network and Advanced. The last two
  open by themselves when they hold values.
- **Test connection**, without saving:
  - SSH logs in through jump hosts, with no shell and no command. An unknown host key is
    reported with its fingerprint and never accepted.
  - Databases connect and close.
  - RDP only checks that the server answers, and says so.
- Operations history settings in Settings › Approvals.

### Accessibility
- Visible focus, labelled controls, keyboard access to the new views.
- Screen-reader announcements for filters, agent status and test results.
- Reduced motion is respected.
- The PROD label does not rely on colour alone.

### Internationalisation
- Italian translation of the whole interface, with English as the fallback.
- Language switch without restarting.

### Performance
- Connection filtering stays well under 25 ms for 2000 connections (unit test).
- File listings render in steps of 300 rows: first paint for a 5000-entry folder went from about
  1.2 s to about 0.12 s, measured in the browser preview.
- The activity timeline is built from tool and approval events, not from every streamed token.

### Fixes
- In the dual-pane file manager, selecting a row no longer makes the toolbar taller, which
  moved the rows so that a double-click opened the row above.
- A restored workspace no longer opens a local shell in place of a deleted connection.
- Approval requests withdrawn by the backend no longer stay "pending" in the sidebar.
- Uploads over an existing file now ask first, with a comparison. They used to replace it
  silently.
- The SQL import skips psql meta-commands (`\connect`, `\restrict`) instead of sending them to
  the server.
