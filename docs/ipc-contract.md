# UI ⇄ Core IPC contract (v1)

Tauri `invoke` commands (snake_case) and events. Payload types mirror `crates/aht-core` (serde
snake_case; ids are UUID strings). The UI keeps a typed wrapper in `ui/src/lib/api/` with a mock
implementation used when not running inside Tauri (`window.__TAURI_INTERNALS__` absent).

## Vault
- `vault_status() -> { initialized: bool, locked: bool }`
- `vault_create({ password }) -> { secret_key: string }` — Secret Key shown once (Emergency Kit)
- `open_url({ url }) -> ()` — opens an https link in the default browser (the webview ignores
  `target="_blank"`); anything but plain https is refused
- `save_emergency_kit({ contents }) -> string` — writes `Handrail Emergency Kit.txt` to Downloads
  (never overwrites; owner-only on Unix) and returns the path. The webview cannot save `blob:` downloads.
- `vault_unlock({ password, secret_key? }) -> void` (secret_key needed only on a new device)
- `vault_lock() -> void`
- `vault_delete() -> void` — deletes the vault from this device, locked or not: items, snapshots,
  the Secret Key in the keychain, the sync settings and the AI profiles' folders; closes every
  session. Synced copies (GitHub, cloud folders) and known_hosts are kept. Status is then
  uninitialized.

## Remote desktop
RDP sessions run in a `handrail-rdp` helper process (rdp-helper/), one per session. Desktop
updates reach the UI as raw messages on a channel: `[kind u8][payload]`, little-endian.
- `0x11` tiles: `[u16 width][u16 height][u16 count]`, then per rectangle
  `[u16 x][u16 y][u16 w][u16 h]` and its RGBA pixels row by row
- `0x12` pointer: `[0]` default, `[1]` hidden, or `[3][u16 w][u16 h][u16 hot_x][u16 hot_y][RGBA]`
  (premultiplied alpha)
- `0x13` error: JSON `{ message }` (the connection failed or the helper stopped)
- `0x14` terminated: JSON `{ reason, error }`

Commands:
- `rdp_open({ connection_id, size: { width, height, scale }, on_event: Channel }) -> string`:
  an `rdp` connection with a saved password; jump hosts become an SSH local forward. Size in
  device pixels, scale in percent.
- `rdp_input({ id, events: RdpInput[] }) -> void`: `{t:"move",x,y}`, `{t:"down"|"up",button}`
  (DOM numbering), `{t:"wheel",vertical,units}` (120 = a notch, positive = up/right),
  `{t:"key",code,down}` (set-1 scancode, extended keys `0xE0xx`), `{t:"release_all"}`,
  `{t:"clipboard",text}` (text pasted into the session, offered to the server before the input
  that follows; not used on Windows, where the clipboard is shared natively)
- `rdp_resize({ id, size }) -> void`, `rdp_close({ id }) -> void`

## Connections & groups
- `list_connections() -> Connection[]`, `save_connection({ connection }) -> Connection`,
  `delete_connection({ id })`
- `list_groups() -> Group[]`, `save_group({ group }) -> Group`, `delete_group({ id })`
- `save_secret({ secret }) -> SecretId` (value never returned by any list command)
- `list_secrets() -> { id, kind, label }[]` (no values), `update_secret({ id, label?, value? })`
  (rename, or replace the value for every connection using it),
  `delete_secret({ id })` (`invalid` while a connection, identity, S3 account or provider uses it)
- `list_identities() -> Identity[]`, `save_identity({ identity }) -> Identity`,
  `delete_identity({ id })`. `Identity`: `{ id, label, user, auth }`, `auth` a `SshAuth`
  (`password`, `key` or `agent`, never `prompt`; `invalid` without a label or user, `not_found`
  when its secret is gone). `Connection.identity?`: log in as that identity (its user and auth
  replace the connection's at connect time). `delete_identity` is `invalid` while a connection
  uses it; its secrets stay in the vault.
- `connection_test({ connection, password?, key?, passphrase? }) -> { status, message?, host?,
  fingerprint?, elapsed_ms }`: logs in to `connection` as edited (need not be saved) and closes
  again; nothing runs on the server. Typed secrets replace the saved ones for this test only.
  `status`: `success` | `reachable` (RDP: answered, credentials not checked) | `auth_failed` |
  `unreachable` | `timeout` (15 s for the whole test) | `host_key_unknown` (never accepted here;
  `host` is `host:port`, with `fingerprint`) | `host_key_mismatch` | `needs_password` | `error`.
  Never rejects for a failed login.
- `Connection.tunnels`: `{ kind: "local"|"remote"|"dynamic", bind_host, bind_port, dest_host,
  dest_port, enabled }[]` (ssh -L / -R / -D; `bind_port` 0 = any free port)

## Sessions
- `open_session({ connection_id, cols, rows, prompt_ref? }) -> SessionInfo`: with `prompt_ref`,
  a connection without saved credentials asks for them while logging in: event
  `session://auth_prompt` `{ request_id, prompt_ref, name, instructions, prompts: { prompt, echo
  }[] }`, answered with `auth_prompt_answer({ request_id, answers: string[] | null })` (`null`
  cancels the login, as does no answer within 5 min)
- `open_local_session({ shell?, cols, rows }) -> SessionInfo`
- `session_write({ session_id, data_b64 })`, `session_resize({ session_id, cols, rows })`,
  `close_session({ session_id })`, `list_sessions() -> SessionInfo[]`
- Port forwards start in the background with each SSH session and stop with it:
  `session_tunnels({ session_id }) -> { tunnel, port, error }[]` (neither port nor error: still
  starting), then event `session://tunnels` `{ session_id, tunnels }` once they started or failed
- `session_stats({ session_id }) -> { cpu, cores, mem_used, mem_total, disk_used, disk_total,
  load1, uptime_s }` (from /proc on its own channel, ~0.5 s; null where the server has no value)
- `SessionInfo.ended` (once `disconnected`): `{ exit_status, error }`. An exit status means the
  shell ended (e.g. `exit`); none means the connection was lost, and the UI then logs in again by
  itself (SSH with a saved password, key or agent; never on login or host key errors).
- Events: `session://data` `{ session_id, data_b64 }`, `session://state` `SessionInfo`,
  `session://agent_mark` `{ session_id, command, actor_label }`,
  `session://host_key` `{ request_id, host, port, fingerprint, old_fingerprint?, kind: "new" |
  "changed" }` →
  answer with `host_key_decision({ request_id, accept: bool })`

## Approvals
- Event `approval://requested` `ApprovalRequest`; answer `approval_decide({ approval_id, decision })`.
  `approval://resolved` (approval id) when a request ended without an answer (timed out, run
  cancelled): denied.
- `set_approval_mode({ session_id, mode })`

## Files (SFTP)
SFTP runs on the SSH connection of an open SSH session (no second login); local sessions refuse.
- `sftp_home({ session_id }) -> string`, `sftp_list({ session_id, path }) -> FileEntry[]`
  (`{ name, path, kind: "dir"|"file"|"symlink"|"other", link, size, modified, permissions }`,
  folders first; `link`: a symlink, `kind` is then its target's, `symlink` when that is missing)
- `sftp_mkdir({ session_id, path })`, `sftp_rename({ session_id, from, to })`,
  `sftp_remove({ session_id, path, is_dir })` (empty folders only)
- `sftp_download({ session_id, path, transfer_id }) -> local path` (into Downloads, never
  overwrites), `sftp_upload({ session_id, local_path, dir, transfer_id }) -> remote path`
- `copy_between({ from, path, to, to_dir, overwrite, transfer_id }) -> new path`, where a place is
  `{ kind: "sftp", session_id, root? }`, `{ kind: "s3", account_id }` or `{ kind: "local" }`
  (this computer, see below): server to server (files and folders, permission bits kept), S3 to
  a server and a server to S3 (files; streamed, nothing saved locally); files between this
  computer and a server or S3, and inside this computer. S3 to S3 is `invalid`. `busy`/`exists`
  when the name is taken and not `overwrite` (files replaced, folders merged; a replaced file on
  a server keeps its permissions)
- Event `transfer://progress` (SFTP and S3) `{ transfer_id, done, total }` (at most ten a second, plus the last)
- Every `sftp_*` command takes `root?: bool`: use the session's root server, started by
  `sftp_sudo({ session_id, password? })` (sudo on its own channel: `sudo -n`, then the connection's
  saved login password, else it rejects with `auth`/`sudo_password_required` and the UI asks; the
  password is used once and never stored); `sftp_root_off({ session_id })` when the file browser
  leaves root mode (agent proposals go back to the login user; the root server stays for editors
  opened as root)
- Editor: `sftp_read_text({ session_id, path, root? }) -> { text, stamp: { size, modified, hash }
  }` (UTF-8 up to 5 MB, binaries refused; `hash` is the SHA-256 of the contents, hex),
  `sftp_write_text({ session_id, path, text, expected, backup, root? }) -> stamp` (in place;
  `expected` null for a new file; `busy`/`changed_on_server` when the file changed since
  `expected`, compared by `hash`; `backup` copies it to `<path>.bak` first)
- Agent tool `propose_file_edit { session_id, path, content, reason? }` (built-in agent and MCP):
  event `edit://proposal` `{ proposal_id, session_id, path, root, original, stamp, proposed,
  reason, actor_label }`, answered with `edit_proposal_decide({ proposal_id, outcome:
  "saved"|"rejected"|"failed", note? })` after the UI saved it; `edit://resolved` (id) when the
  agent stopped waiting
- Upload paths come from the native file dialog (`tauri-plugin-dialog`, `dialog:allow-open` only).

## This computer (local files)
The file manager's local pane. Paths use slashes in both directions: the native path on macOS and
Linux, `/C:/Users/me` for `C:\Users\me` on Windows, whose `/` lists the drives. Paths must be
absolute and hold no `..` (`invalid`).
- `local_home() -> string`, `local_list({ path }) -> FileEntry[]`, `local_mkdir({ path })`
- `local_rename({ from, to })` (never replaces: a taken name is `busy`/`exists`)
- `local_remove({ path })` (a folder goes with everything it holds; a link removes only the link)
- `local_read_text({ path }) -> { text, stamp }` (UTF-8 up to 5 MB, for the comparison before a
  copy replaces a file)
- Copies to and from this computer go through `copy_between` with `{ kind: "local" }`.

## S3
Accounts are vault items; the secret access key is a vault secret and never comes back over IPC.
- `list_s3_accounts() -> S3Account[]`, `save_s3_account({ account, secret_key? })` (a key replaces
  the stored one), `delete_s3_account({ id })` (also deletes the secret)
- Paths are `/bucket/folder/file`; `/` lists the buckets. `s3_home({ account_id })`,
  `s3_list({ account_id, path }) -> FileEntry[]`, `s3_mkdir` (an empty `folder/` object),
  `s3_rename({ account_id, from, to, is_dir })` (files only: copy + delete),
  `s3_remove({ account_id, path, is_dir })` (empty folders only), `s3_download`, `s3_upload`
  (single PUT, up to 5 GiB) with the same arguments and progress events as SFTP.
- `s3_read_text({ account_id, path }) -> { text, stamp: { size, modified: null, hash } }`: an
  object as text for the editor, read-only (S3 has no in-place edits); same 5 MB and text checks
  as SFTP.
- SigV4-signed requests to AWS (virtual-hosted, bucket region discovered with HEAD) or any
  S3-compatible endpoint (path-style optional).

## Databases
MySQL/MariaDB and PostgreSQL tabs. Each tab (`id`) holds two connections (one runs the user's
statements, so `BEGIN`, `USE`, `SET` carry over between runs; the other reads the schema and
cancels) and, for a connection with jump hosts, an SSH tunnel; all close with the tab. Values
arrive as text: a `Value` is `null`, a string, or `{ hex }` for bytes that are not UTF-8. A
`ResultSet` is `{ columns: { name, type_name }[], rows: Value[][], rows_affected, truncated }`.
- `db_open({ connection_id, password? }) -> { id, kind: "mysql"|"postgres", version, database? }`
  (`password`: typed at the prompt of a connection that saves none; without it such a connection
  rejects `{ kind: "auth", message: "password_required" }`), `db_close({ id })` (also denies the
  tab's pending agent SQL and stops its exports and imports)
- `db_databases({ id }) -> string[]` (PostgreSQL: the ones a new tab could open),
  `db_namespaces({ id }) -> string[]` (databases on MySQL, schemas on PostgreSQL)
- `db_tables({ id, namespace }) -> { name, view, materialized, rows?, size?, engine?, comment?
  }[]` (`rows`: the server's estimate), `db_columns({ id, namespace, table }) -> { name,
  data_type, nullable, default?, primary_key }[]`
- `db_query({ id, sql, max_rows? }) -> { results: ResultSet[], error?, elapsed_ms }` (one or more
  statements; `max_rows` per result, default 1000, at most 100 000), `db_cancel({ id })` (stops
  the running statement)
- `db_table_rows({ id, namespace, table, max_rows?, offset?, order_by?, desc?, filter? })` →
  same output: a page of one table, sorted by a column and filtered by a `WHERE` condition
- `db_table_structure({ id, namespace, table }) -> { columns: { name, data_type, nullable,
  default?, primary_key, extra?, comment?, collation? }[], indexes: { name, columns, unique,
  primary, kind }[], foreign_keys: { name, columns, ref_namespace, ref_table, ref_columns,
  on_update, on_delete }[] }`, `db_table_ddl({ id, namespace, table }) -> string` (the `CREATE`
  of a table or view)
- `db_apply_edits({ id, namespace, table, edits }) -> number` (rows changed): grid edits in one
  transaction, rows found by primary key (a table without one is refused). An edit is `{ op:
  "update", key, set }`, `{ op: "insert", values }` or `{ op: "delete", key }`, each a list of
  `[column, string | null]` pairs.
- Server panel: `db_server_info({ id, what: "processes"|"variables"|"status"|"users" }) ->
  ResultSet`, `db_kill({ id, pid, connection })` (stop the process's statement, or close its
  connection), `db_grants({ id, user, host? }) -> ResultSet`
- Accounts: `db_account_preview({ id, change }) -> string[]` (the statements, password masked),
  `db_account({ id, change })`. `change` is `{ op: "create"|"password", user, host?, password }`,
  `{ op: "drop", user, host? }` or `{ op: "grant"|"revoke", user, host?, privileges: string[],
  namespace? }` (`host`: MySQL's account host, `%` when empty; `namespace` null: everything,
  MySQL only). Only known privilege names are accepted.
- Export: `db_export_file({ file_name, contents }) -> path | null` (rows already rendered by the
  UI, written to a file picked in a save dialog; `file_name` is only the suggested name),
  `db_dump({ id, namespace, options: { tables?, structure, data, drop? }, transfer_id }) -> {
  path, summary } | null` (an SQL file picked in a save dialog; `tables` empty: all; null when the
  dialog is cancelled; a stopped export returns `path: ""` and a failed one rejects, neither
  leaving a file)
- Import: `db_pick_file({ kind: "sql"|"csv" }) -> { token, name, size } | null` (the page holds the
  token, never the path; an import that completes uses it up, a failed one keeps it for a retry),
  `db_csv_preview({ token, delimiter }) -> string[][]` (first records),
  `db_import_sql({ id, token, namespace?, options: { stop_on_error }, transfer_id }) -> Summary`
  (a file that is not UTF-8 is refused before any of it runs; on PostgreSQL `stop_on_error` runs
  the file in one transaction), `db_import_csv({ id, token,
  namespace, table, options: { delimiter, header, null?, columns: (string|null)[], replace? },
  transfer_id }) -> Summary` (`columns`: the column of each field, null skips it; `replace`
  deletes the rows first; all of it or nothing)
- `Summary`: `{ progress, errors: { line, statement, message }[] (the first 50), cancelled,
  rolled_back }`; `progress`: `{ table?, tables_done, tables, rows, statements, bytes, errors }`.
  Event `db://transfer` `{ transfer_id, progress }` (at most five a second);
  `db_transfer_cancel({ transfer_id })` stops an export, import or backup indexing.
- SQL backups without a server: `db_backup_open({ kind?: "mysql"|"postgres", transfer_id }) -> {
  token, name, size, compressed, index } | null` (picks a `.sql` or `.sql.gz` file and indexes it;
  no `kind`: the file's first lines decide; a `pg_dump` custom archive is `invalid`), `index`:
  `{ kind, tables: { schema?, name, view, columns, rows, create? }[], statements, bytes }`;
  `db_backup_reindex({ token, kind, transfer_id }) -> index` (read again as the other dialect);
  `db_backup_rows({ token, table, from, limit?, filter?, column? }) -> { columns, rows: Value[][],
  positions, next? }` (`table`: position in `index.tables`; `limit` default 500, at most 10 000;
  `filter`: rows holding that text, in `column` or any); `db_backup_export({ token, table, format:
  "csv"|"json"|"sql", filter?, column?, transfer_id }) -> { path, rows } | null` (save dialog);
  `db_backup_close({ token })`
- Agents: `db_share({ id, shared })` (off by default; agents can then read the tab's schema and
  query it through a connection of their own; unsharing denies its pending SQL). Agent SQL is
  classified `read`/`write`/`destructive`: destructive always waits, writes wait unless the mode
  is `auto` (always on production), reads wait only under `confirm_all`. Event `db://approval` `{
  approval_id, database_id, actor_label, sql, reason?, risk, statements, policy_note }`, answered
  with `db_approval_decide({ approval_id, decision })` (`not_found` once resolved; an edited
  approval counts as a denial); `db://approval-resolved` (approval id) when the wait ended for any
  reason (no answer within 15 min denies).

## Snippets
- `list_snippets()`, `save_snippet({ snippet })` (name and command required), `delete_snippet({ id })`.
  Vault item type `snippet` `{ id, name, command, description, tags }`; `{{name}}` placeholders are
  filled in by the UI before the command is typed into the terminal.

## Providers, harnesses, agent
- `list_providers()`, `save_provider({ provider, api_key? })`, `delete_provider({ id })`,
  `list_models({ provider_id }) -> string[]`
- `detect_harnesses() -> { kind, path?, version?, logged_in?: bool }[]`
- `list_harness_profiles()`, `save_harness_profile({ profile })`, `delete_harness_profile({ id })`
- `list_harness_extensions({ kind }) -> { skills: {name, path}[], plugins: {name, path}[] }`
- `agent_send({ session_id, brain, message }) -> { run_id }`, `agent_cancel({ run_id })`,
  `agent_reset({ session_id })` (forgets the tab's built-in agent conversation)
- Event `agent://event` `{ run_id, session_id, event: AgentEvent }`
- `harness_open_native({ session_id, profile_id, cols, rows }) -> SessionInfo` (CLI TUI in a PTY tab)
- `harness_login({ kind, cols, rows }) -> SessionInfo` (runs the CLI's own login in a local tab)
- `openchamber_open({ session_id, profile_id }) -> void`: an OpenCode profile (`invalid`
  otherwise) in OpenChamber, its web UI: a private OpenChamber server on 127.0.0.1 with a random
  password and Handrail's tools, shown in a window of its own. Closing the window stops the
  server and revokes the tools' token.

## Import & sync
- `import_detect() -> { id, tool, path?, count_hint? }[]`
- `import_preview({ args: { source_id? , tool?, file_path?, mremoteng_password?, import_passwords? } })
  -> { preview_id, tool, groups, connections, secret_count, warnings, duplicates: { index, existing_id }[] }`
  (bundle and secret values stay in the backend)
- `import_apply({ preview_id, skip_indices: number[] }) -> { imported }`
- `sync_status() -> { config?: {kind:"github",owner,repo,branch}|{kind:"folder",path}, last_sync_ms?, last_report?, can_join,
  auto, syncing, last_error? }`
- Event `sync://status` (same payload as `sync_status`) when a sync starts or ends and when settings change.
  While the vault is unlocked and `auto` is on, the backend syncs after unlock, ~3 s after a local edit
  and every 5 min; failures back off from 30 s to 15 min.
- `sync_set_auto({ enabled })`, `sync_disconnect()` (this device only; vault and remote untouched)
- `sync_folder_presets() -> { drive: "icloud"|"dropbox"|"onedrive"|"google_drive", label, path, has_vault }[]`
  (cloud-drive folders found on this device; `path` is `<drive>/Handrail`)
- `sync_configure_github_start() -> { device_code, user_code, verification_uri, interval, expires_in }`
- `sync_configure_github_poll() -> { done, status: "pending"|"slow_down"|"expired"|"denied"|"ok", repo? }`
  (poll every `interval` s; creates private repo `handrail-vault`)
- `sync_configure_folder({ path })`, `sync_join()` (new device: pull manifest, then unlock with Secret Key;
  rejects `{ kind: "not_found" }` when the target holds no vault yet),
  `sync_now() -> { pulled, pushed, conflicts, warnings }`

## Snapshots & conflict history
- `vault_snapshots() -> { name, created_ms, size }[]` (newest first; max 10 kept)
- `vault_snapshot_create() -> { name, created_ms, size }`, `vault_snapshot_restore({ name })`
  (takes a safety snapshot first; the rollback is written as fresh edits so it wins on sync).
  Each sync takes a snapshot first and keeps it only if the sync changed local files.
- `vault_conflicts() -> { file_id, recorded_at_ms, item_type, label, loser_ms, loser_device, loser_deleted, winner_ms }[]`
  (versions discarded by last-writer-wins merges, newest first; no payloads)
- `vault_conflict_restore({ file_id, recorded_at_ms })` (re-applies the discarded version as a new edit),
  `vault_conflicts_clear()`

## App & updates
- `app_version() -> string` (workspace `Cargo.toml` version; release builds stamp `<major>.<minor>.<build>`)
- `restart_app()` (after an installed update on macOS/Linux)
- `nightly_build() -> { version, page_url, published_at?, download?: { name, url, size } } | null`:
  the newest Handrail Nightly prerelease (found through `nightly-latest/latest.json`), with the
  installer for this system if it has one; `null` when none is published. Only offered for
  download in the browser, never installed by the app.
- Updates use `@tauri-apps/plugin-updater` (`updater:default` capability) against
  `https://github.com/andreafunicelli/Handrail/releases/latest/download/latest.json`.

## Misc
- `get_setting({ key }) -> any|null`, `set_setting({ key, value })`
- Errors reject with `{ kind, message }` (aht-core `Error`); `vault_unlock` rejects
  `{ kind: "auth", message: "secret_key_required" }` when this device needs the Secret Key.
