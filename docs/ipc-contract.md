# UI ⇄ Core IPC contract (v1)

Tauri `invoke` commands (snake_case) and events. Payload types mirror `crates/aht-core` (serde
snake_case; ids are UUID strings). The UI keeps a typed wrapper in `ui/src/lib/api/` with a mock
implementation used when not running inside Tauri (`window.__TAURI_INTERNALS__` absent).

## Vault
- `vault_status() -> { initialized: bool, locked: bool }`
- `vault_create({ password }) -> { secret_key: string }` — Secret Key shown once (Emergency Kit)
- `vault_unlock({ password, secret_key? }) -> void` (secret_key needed only on a new device)
- `vault_lock() -> void`

## Connections & groups
- `list_connections() -> Connection[]`, `save_connection({ connection }) -> Connection`,
  `delete_connection({ id })`
- `list_groups() -> Group[]`, `save_group({ group }) -> Group`, `delete_group({ id })`
- `save_secret({ secret }) -> SecretId` (value never returned by any list command)

## Sessions
- `open_session({ connection_id, cols, rows }) -> SessionInfo`
- `open_local_session({ shell?, cols, rows }) -> SessionInfo`
- `session_write({ session_id, data_b64 })`, `session_resize({ session_id, cols, rows })`,
  `close_session({ session_id })`, `list_sessions() -> SessionInfo[]`
- Events: `session://data` `{ session_id, data_b64 }`, `session://state` `SessionInfo`,
  `session://agent_mark` `{ session_id, command, actor_label }`,
  `session://host_key` `{ request_id, host, fingerprint, kind: "new" | "changed" }` →
  answer with `host_key_decision({ request_id, accept: bool })`

## Approvals
- Event `approval://requested` `ApprovalRequest`; answer `approval_decide({ approval_id, decision })`
- `set_approval_mode({ session_id, mode })`

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
  (cloud-drive folders found on this device; `path` is `<drive>/AIHubTerm`)
- `sync_configure_github_start() -> { device_code, user_code, verification_uri, interval, expires_in }`
- `sync_configure_github_poll() -> { done, status: "pending"|"slow_down"|"expired"|"denied"|"ok", repo? }`
  (poll every `interval` s; creates private repo `aihubterm-vault`)
- `sync_configure_folder({ path })`, `sync_join()` (new device: pull manifest, then unlock with Secret Key),
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

## Misc
- `get_setting({ key }) -> any|null`, `set_setting({ key, value })`, `list_secrets() -> { id, kind, label }[]`
- Errors reject with `{ kind, message }` (aht-core `Error`); `vault_unlock` rejects
  `{ kind: "auth", message: "secret_key_required" }` when this device needs the Secret Key.
