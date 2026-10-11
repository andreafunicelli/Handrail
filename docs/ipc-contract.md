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

## Connections & groups
- `list_connections() -> Connection[]`, `save_connection({ connection }) -> Connection`,
  `delete_connection({ id })`
- `list_groups() -> Group[]`, `save_group({ group }) -> Group`, `delete_group({ id })`
- `save_secret({ secret }) -> SecretId` (value never returned by any list command)
- `list_secrets() -> { id, kind, label }[]` (no values), `update_secret({ id, label?, value? })`
  (rename, or replace the value for every connection using it),
  `delete_secret({ id })` (`invalid` while a connection, S3 account or provider uses it)
- `Connection.tunnels`: `{ kind: "local"|"remote"|"dynamic", bind_host, bind_port, dest_host,
  dest_port, enabled }[]` (ssh -L / -R / -D; `bind_port` 0 = any free port)

## Sessions
- `open_session({ connection_id, cols, rows }) -> SessionInfo`
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
  `session://host_key` `{ request_id, host, fingerprint, kind: "new" | "changed" }` →
  answer with `host_key_decision({ request_id, accept: bool })`

## Approvals
- Event `approval://requested` `ApprovalRequest`; answer `approval_decide({ approval_id, decision })`
- `set_approval_mode({ session_id, mode })`

## Files (SFTP)
SFTP runs on the SSH connection of an open SSH session (no second login); local sessions refuse.
- `sftp_home({ session_id }) -> string`, `sftp_list({ session_id, path }) -> FileEntry[]`
  (`{ name, path, kind: "dir"|"file"|"symlink"|"other", size, modified, permissions }`, folders first)
- `sftp_mkdir({ session_id, path })`, `sftp_rename({ session_id, from, to })`,
  `sftp_remove({ session_id, path, is_dir })` (empty folders only)
- `sftp_download({ session_id, path, transfer_id }) -> local path` (into Downloads, never
  overwrites), `sftp_upload({ session_id, local_path, dir, transfer_id }) -> remote path`
- `copy_between({ from, path, to, to_dir, overwrite, transfer_id }) -> new path`, where a place is
  `{ kind: "sftp", session_id, root? }` or `{ kind: "s3", account_id }`: server to server (files
  and folders, permission bits kept), S3 to a server and a server to S3 (files); streamed, nothing
  saved locally; `busy`/`exists` when the name is taken and not `overwrite` (files replaced,
  folders merged)
- Event `transfer://progress` (SFTP and S3) `{ transfer_id, done, total }` (at most ten a second, plus the last)
- Every `sftp_*` command takes `root?: bool`: use the session's root server, started by
  `sftp_sudo({ session_id, password? })` (sudo on its own channel: `sudo -n`, then the connection's
  saved login password, else it rejects with `auth`/`sudo_password_required` and the UI asks; the
  password is used once and never stored)
- Editor: `sftp_read_text({ session_id, path, root? }) -> { text, stamp: { size, modified } }`
  (UTF-8 up to 5 MB), `sftp_write_text({ session_id, path, text, expected, backup, root? }) ->
  stamp` (in place; `busy`/`changed_on_server` when the file changed since `expected`; `backup`
  copies it to `<path>.bak` first)
- Agent tool `propose_file_edit { session_id, path, content, reason? }` (built-in agent and MCP):
  event `edit://proposal` `{ proposal_id, session_id, path, root, original, stamp, proposed,
  reason, actor_label }`, answered with `edit_proposal_decide({ proposal_id, outcome:
  "saved"|"rejected"|"failed", note? })` after the UI saved it; `edit://resolved` (id) when the
  agent stopped waiting
- Upload paths come from the native file dialog (`tauri-plugin-dialog`, `dialog:allow-open` only).

## S3
Accounts are vault items; the secret access key is a vault secret and never comes back over IPC.
- `list_s3_accounts() -> S3Account[]`, `save_s3_account({ account, secret_key? })` (a key replaces
  the stored one), `delete_s3_account({ id })` (also deletes the secret)
- Paths are `/bucket/folder/file`; `/` lists the buckets. `s3_home({ account_id })`,
  `s3_list({ account_id, path }) -> FileEntry[]`, `s3_mkdir` (an empty `folder/` object),
  `s3_rename({ account_id, from, to, is_dir })` (files only: copy + delete),
  `s3_remove({ account_id, path, is_dir })` (empty folders only), `s3_download`, `s3_upload`
  (single PUT, up to 5 GiB) with the same arguments and progress events as SFTP.
- SigV4-signed requests to AWS (virtual-hosted, bucket region discovered with HEAD) or any
  S3-compatible endpoint (path-style optional).

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
- `agent_send({ session_id, brain, message }) -> { run_id }`, `agent_cancel({ run_id })`
- Event `agent://event` `{ run_id, session_id, event: AgentEvent }`
- `harness_open_native({ session_id, profile_id, cols, rows }) -> SessionInfo` (CLI TUI in a PTY tab)
- `harness_login({ kind, cols, rows }) -> SessionInfo` (runs the CLI's own login in a local tab)

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
- Updates use `@tauri-apps/plugin-updater` (`updater:default` capability) against
  `https://github.com/andreafunicelli/Handrail/releases/latest/download/latest.json`.

## Misc
- `get_setting({ key }) -> any|null`, `set_setting({ key, value })`
- Errors reject with `{ kind, message }` (aht-core `Error`); `vault_unlock` rejects
  `{ kind: "auth", message: "secret_key_required" }` when this device needs the Secret Key.
