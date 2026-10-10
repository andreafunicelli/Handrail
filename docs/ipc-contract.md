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
- `import_detect() -> { source_id, tool, path, count }[]`
- `import_preview({ source_id } | { tool, file_path }) -> ImportBundle`
- `import_apply({ bundle, skip_ids: string[] }) -> { imported: number }`
- `sync_status()`, `sync_configure_github_start() -> { user_code, verification_uri }`,
  `sync_configure_github_poll() -> { done: bool }`, `sync_configure_folder({ path })`, `sync_now()`
