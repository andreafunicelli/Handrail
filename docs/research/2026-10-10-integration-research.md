# AIHubTerm integration research (2026-10-10)

Legend: [V] verified against official docs/registry today; [U] uncertain / from memory / secondary source.

---------------------------------------------------------------------------
## 1. Claude Code (CLI v2.1.29x era)

Sources: https://code.claude.com/docs/en/cli-reference , /headless , /mcp , /permissions , /skills , /authentication , /legal-and-compliance

### (a) MCP config at launch [V]
```
claude --mcp-config ./aihub-mcp.json --strict-mcp-config
```
- `--mcp-config` takes JSON files OR inline JSON strings, space-separated. `--strict-mcp-config` = only use servers from `--mcp-config`, ignore all other MCP config (user, project `.mcp.json`, claude.ai connectors).
- With `-p`, Claude waits for pending MCP servers (up to `MCP_TIMEOUT`, default 30 s) before the first turn (v2.1.221+).
- Entries need an explicit `type` (a `url` entry with no `type` is skipped). Failures show in `system/init` -> `mcp_server_errors`; loaded ones in `mcp_servers[].status`.
- Config shape (env expansion `${VAR}` / `${VAR:-default}` works in `url` and `headers`), so the bearer token need not be written to disk or argv:
```json
{ "mcpServers": { "aihub": {
    "type": "http",
    "url": "http://127.0.0.1:54321/mcp",
    "headers": { "Authorization": "Bearer ${AIHUB_MCP_TOKEN}" } } } }
```
  Spawn the child with env `AIHUB_MCP_TOKEN=<per-launch token>`. Caveat: vars named `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `AWS_BEARER_TOKEN_BEDROCK` read as empty in headers; use your own var name.
- Windows: pass inline JSON carefully (quote escaping); a temp file in app-data with user-only ACL is simpler. Server name `aihub` -> tools are `mcp__aihub__<tool>`.

### (b) Headless streaming [V]
```
claude -p --output-format stream-json --input-format stream-json --verbose [--include-partial-messages] [--replay-user-messages]
```
- `--output-format stream-json` needs `--verbose` (docs' own examples always pair them). `--include-partial-messages` adds `stream_event` token deltas (`event.delta.type == "text_delta"`).
- Event stream: first `system/init` (model, tools, `mcp_servers`, `plugins`, `plugin_errors`, optional `capabilities[]`), then `assistant`/`user` messages (tool_use/tool_result blocks; subagent msgs carry `parent_tool_use_id`), `system/api_retry`, `permission_denied` system msgs, last line = `result` (text, cost, session_id, `permission_denials`).
- `--input-format stream-json`: one JSON user message per stdin line. Exact shape not on the CLI page [U, from Agent SDK knowledge]: `{"type":"user","message":{"role":"user","content":[{"type":"text","text":"..."}]}}` ; verify with `--replay-user-messages` against the installed version. Process stays alive for multi-turn.
- Other useful: `--session-id <uuid>`, `--resume`, `--max-turns`, `--max-budget-usd`, `--no-session-persistence`, `--permission-mode`, `--permission-prompts none` (v2.1.259+; deny instead of waiting), `--permission-prompt-tool <mcp tool>` (route approvals to our MCP server).
- SIGTERM -> exit 143; SIGINT ends the turn cleanly.
- `--bare` = skip hooks/skills/plugins/MCP/CLAUDE.md autodiscovery BUT never reads OAuth/keychain (needs ANTHROPIC_API_KEY or apiKeyHelper). So **`--bare` is incompatible with subscription login**.

### (c) System prompt and tool restriction [V]
- `--append-system-prompt "<text>"` / `--append-system-prompt-file <f>` (append, keeps defaults). `--system-prompt[-file]` replaces entirely. Can combine flag + its file form from v2.1.283.
- Tool names for MCP: `mcp__<server>__<tool>`. Rule forms (permissions page): `mcp__aihub` (whole server), `mcp__aihub__*` (glob after literal server prefix), `mcp__aihub__ssh_exec` (one tool). Allow-globs not anchored to a literal `mcp__<server>__` prefix are skipped; parentheses rules on `mcp__` not valid in settings files.
- Auto-approve our tools: `--allowedTools "mcp__aihub__*"` (variadic; comma or space separated).
- Remove built-ins: `--tools ""` disables ALL built-in tools (does not affect MCP tools), or `--disallowedTools "Bash" "Edit" "Write" "NotebookEdit"` (a bare name removes the tool from context; scoped `Bash(rm *)` only denies matches). `--disallowedTools "mcp__*"` removes all MCP tools. Recommended for "agent only acts through the SSH terminal": `--tools "Read,Grep,Glob"` or `--tools ""` plus `--allowedTools "mcp__aihub__*"` and `--permission-mode dontAsk` (denies anything that would prompt).
- `--restricted` (v2.1.248+): removes command/code-running tools + WebFetch, confines file tools, loads only managed settings and `--settings`. Worth testing as a one-flag lockdown.

### (d) Plugins, skills, setting sources [V]
- `--plugin-dir <dir|.zip>` (repeatable; a folder of plugins accepted from v2.1.265): session-only plugin. Plugin layout `<plugin>/.claude-plugin/plugin.json` (manifest) [U on exact manifest path - check plugin docs], `<plugin>/skills/<name>/SKILL.md`. Plugin skills are namespaced `/plugin-name:skill-name`, never collide. Load results visible in `system/init` `plugins`/`plugin_errors`. `--plugin-url <zip-url>` also exists.
- Skill discovery: enterprise (managed dir) > personal `~/.claude/skills/<n>/SKILL.md` > project `.claude/skills/<n>/SKILL.md` (launch dir up to repo root); nested `<sub>/.claude/skills`; `--add-dir` dirs' `.claude/skills`; plugin skills. SKILL.md frontmatter: `name`, `description`, `when_to_use`, `disable-model-invocation`, `user-invocable`, `allowed-tools`, `context: fork`, `agent`, `paths`. Keep SKILL.md < 500 lines.
- `--setting-sources user,project,local` (comma list): which settings layers load. To isolate from repo-level settings/hooks use `--setting-sources user` (or `""` [U whether empty is accepted]); pair with `--settings <file|json>` to inject our own. Note: without `--bare`, `-p` runs project hooks and connects project `.mcp.json` even in untrusted folders (headless doc) -> use `--strict-mcp-config` and `--setting-sources user`.
- `--disable-slash-commands` disables all skills/commands. `--safe-mode` disables all customizations but keeps auth (unlike `--bare`).

### Login detection (non-interactive) [V]
`claude auth status` -> JSON, exit 0 if logged in, 1 if not; `--text` for human output. `authMethod` in {`none`,`claude.ai`,`oauth_token`,`api_key`,`api_key_helper`,`third_party`}; `configDirectory` field (v2.1.268+). Also `claude auth login [--console|--sso|--email]` (spawn it in a visible PTY tab for the user), `claude auth logout`. Credentials live in `%USERPROFILE%\.claude\.credentials.json` (Windows), Keychain/`~/.claude/.credentials.json` (mac/Linux); `CLAUDE_CONFIG_DIR` relocates. **Do not read or copy these files.** Auth precedence: cloud-provider env > ANTHROPIC_AUTH_TOKEN > ANTHROPIC_API_KEY (always used in -p if set!) > apiKeyHelper > CLAUDE_CODE_OAUTH_TOKEN > subscription login. -> When spawning with subscription login, scrub `ANTHROPIC_API_KEY`/`ANTHROPIC_AUTH_TOKEN` from the child env or it silently bills the API.

### (e) POLICY: third-party app driving user's local Claude Code with Pro/Max login
Official statements (verbatim, short):
1. Claude Code legal & compliance, "Authentication and credential use": "Anthropic does not permit third-party developers to offer Claude.ai login into their own applications, or to route requests through Free, Pro, or Max plan credentials on behalf of their users." Also: developers "may not collect, store, or intermediate Claude.ai credentials or session tokens". And "Developers building products or services that interact with Claude's capabilities, including those using the Agent SDK, should use API key authentication". (https://code.claude.com/docs/en/legal-and-compliance)
2. Same page, carve-out: the restriction does not "prevent an end user from signing in to the unmodified Claude Code binary with their own Claude subscription, including where a platform hosts Claude Code". Conditions for products running Claude Code: binary unmodified, auth methods not disabled/restricted, and customers "may not pay for, resell, or intermediate Claude usage on their end users' behalf"; each end user "must authenticate with their own ... Claude subscription plan credentials".
3. Agent SDK overview: "Unless previously approved, Anthropic does not allow third party developers to offer claude.ai login or rate limits for their products, including agents built on the Claude Agent SDK." (https://code.claude.com/docs/en/agent-sdk/overview)
4. Usage: "Advertised usage limits for Pro and Max plans assume ordinary, individual usage of Claude Code and the Agent SDK." Enforcement "may [be done] without prior notice."
5. Billing status: Help Center article (https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan) banners: the announced June-15 change was paused; Oct 7 2026: "You can still use the Claude Agent SDK, claude -p, and third-party apps with your subscription limits." (Treat as current but changeable.)
6. Consumer Terms apply to Free/Pro/Max (https://www.anthropic.com/legal/consumer-terms) - not re-read in full today [U].

Analysis / recommendation:
- Pattern A - "AIHubTerm launches the user's own installed, unmodified `claude` (interactive PTY tab or `-p`), user logged in themselves via `claude auth login`, we never see/store/forward tokens, no resale": fits the carve-out in (2) and the "native binary" reading. Gray area: the headless/programmatic path (`-p`, stream-json) is literally the Agent SDK surface that point (3) says third-party products may not offer claude.ai login/rate limits for; "ordinary, individual usage" (4) is also vague. Risk is policy ambiguity + server-side enforcement, not a technical block (Claude Code binary itself authenticates normally).
- Pattern B - extracting/forwarding OAuth tokens, `claude setup-token` tokens injected as CLAUDE_CODE_OAUTH_TOKEN by our app, using the Agent SDK/OAuth in-process, or our built-in agent using the subscription: NOT allowed. Never do this.
- **Recommendation**: (1) Ship Claude Code as an opt-in "Use my installed Claude Code" integration: detect via `claude --version` + `claude auth status`; if not logged in, open a PTY tab running `claude auth login` and let Anthropic's own flow complete. (2) Prefer launching it in an interactive terminal tab (plainly "user runs Claude Code") with `--mcp-config/--strict-mcp-config`; offer headless `-p` as an "advanced" mode with a one-time disclaimer that the user's own plan limits apply and that Anthropic's terms govern. (3) Never read `.credentials.json`, never set CLAUDE_CODE_OAUTH_TOKEN, never use `--bare` for subscription users, don't modify the binary, don't brand as "Claude Code" (SDK branding rules forbid "Claude Code"/"Claude Code Agent" names and look-alike art; say "runs Claude Code"). (4) Offer Console API-key / Bedrock / Vertex as the supported path (document `ANTHROPIC_API_KEY` option) and make the built-in agent API-key only. (5) Before commercial release, ask Anthropic (contact-sales form linked from the policy page) for written confirmation; policy says "Unless previously approved".

---------------------------------------------------------------------------
## 2. OpenAI Codex CLI

Docs moved: developers.openai.com/codex/* now 308-redirects to https://learn.chatgpt.com/docs/... . Sources: /docs/extend/mcp, /docs/non-interactive-mode, /docs/developer-commands, /docs/config-file/config-reference, /docs/auth.

### MCP at launch [V docs; -c on MCP keys is inferred]
- Config table: `[mcp_servers.<name>]`. Streamable HTTP: `url`, `bearer_token_env_var = "ENVNAME"` (sent as `Authorization: Bearer <env value>`), `http_headers = {..}`, `env_http_headers = { "X-Api-Key" = "ENV" }`, `http_headers_helper` (command printing JSON headers). Stdio: `command`, `args`, `cwd`, `env_vars`, `[...env]`. Also `enabled`, `required` (fail startup if server cannot init; `codex exec` exits with error), `startup_timeout_sec` (10), `tool_timeout_sec` (60), `enabled_tools`, `disabled_tools`, `default_tools_approval_mode = auto|prompt|writes|approve`, `tools.<tool>.approval_mode`. Server `auth` default `oauth`; for static bearer set `bearer_token_env_var` (and avoid OAuth discovery).
- Global flag `-c/--config key=value`: "values parse as TOML if possible; otherwise literal string"; CLI overrides win over `~/.codex/config.toml`. The MCP doc page itself does not document -c for MCP, but the config-reference lists every key, so dotted overrides work [U, test on installed version]:
```
codex exec --json \
  -c 'mcp_servers.aihub.url="http://127.0.0.1:54321/mcp"' \
  -c 'mcp_servers.aihub.bearer_token_env_var="AIHUB_MCP_TOKEN"' \
  -c 'mcp_servers.aihub.required=true' \
  -c 'mcp_servers.aihub.default_tools_approval_mode="approve"' \
  "prompt"
```
  with child env `AIHUB_MCP_TOKEN`. Windows/PowerShell quoting: TOML strings need the inner double quotes; pass args as an argv array from Rust (no shell) to avoid this. `--profile/-p <name>` layers `$CODEX_HOME/<name>.config.toml`; `CODEX_HOME` env can point to an isolated home (but then ChatGPT login in `auth.json` is not found - copy would be credential handling; avoid). `--ignore-user-config`, `--ignore-rules` exist on exec.
- `codex mcp add|list|get|remove|login|logout` edits `~/.codex/config.toml` (global; avoid). `codex mcp-server` was REMOVED; use `codex app-server`.

### codex exec --json [V]
JSONL on stdout. Types: `thread.started{thread_id}`, `turn.started`, `item.started`/`item.updated`/`item.completed{item:{id,type,...}}`, `turn.completed{usage:{input_tokens,cached_input_tokens,output_tokens,reasoning_output_tokens}}`, `turn.failed`, `error`. Item types: `agent_message{text}`, `reasoning`, `command_execution{command,status}`, `file_change`, `mcp_tool_call`, `web_search`, `plan` update (shape for mcp_tool_call not shown in docs [U]). Other flags: `-o/--output-last-message <file>`, `--output-schema`, `--skip-git-repo-check`, `--ephemeral`, `-C/--cd`, `-m`, prompt via arg or stdin (`codex exec -`), `codex exec resume --last|<SESSION_ID>`.

### Disable local shell [V partial]
- `--sandbox/-s read-only|workspace-write|danger-full-access` (exec default read-only). `--ask-for-approval/-a on-request|never` (config-reference: `untrusted` unsupported, `on-failure` deprecated, also granular table). Use `-s read-only -a never`: commands still run but read-only/no network, never prompt.
- To remove the shell tool entirely: `-c 'features.shell_tool=false'` (documented as "Enable the default shell tool"; docs don't promise it blocks all exec, e.g. unified_exec `features.unified_exec=false` too) [U - test that the model then has only MCP tools]. Also `-c 'web_search="disabled"'`.

### Login status [V]
`codex login status` prints auth mode, exit 0 when logged in. Creds in `~/.codex/auth.json` or OS keyring (`cli_auth_credentials_store = file|keyring|auto|ephemeral`). `codex exec` uses saved auth; `CODEX_API_KEY` env overrides per run. `codex login` (browser) / `--device-auth` [U].

### app-server (JSON-RPC over stdio JSONL/ws) [V]
`codex app-server [--listen stdio://|ws://IP:PORT|unix://PATH|off]` - marked experimental, "may change without notice"; threads (`thread/resume`), turns (`completed|failed|interrupted`), `model/list`, `clientInfo{name,title,version}`. Richer than `exec --json` (approvals, streaming, interrupt). Pin Codex version if used. Schema generation command (`codex app-server generate-ts`/`generate-json-schema`) exists [U].

### Policy: third-party apps + ChatGPT login [partially V]
- No explicit prohibition found on spawning the user's own `codex` with their own ChatGPT login. Auth doc: auth.json tokens are "intended for trusted scripts, schedulers, and private CI runners"; treat like a password; general API use should use Platform API keys. (https://learn.chatgpt.com/docs/auth)
- OpenAI has an official third-party route: app-server "token sharing" where the app obtains the user's ChatGPT OAuth token via its own registered client ID ("Request a client ID", "Registration and sign-in") and passes it to app-server (https://developers.openai.com/siwc/token-sharing-open-source/codex-app-server). That requires registration; don't scrape `auth.json` yourself.
- Recommendation: same pattern as Claude: launch user's installed `codex`, let the user run `codex login`, never read auth.json; API-key option as fallback. Apply to Business Terms/consumer ToU review [U - not retrieved].

---------------------------------------------------------------------------
## 3. OpenCode (sst/opencode -> opencode.ai)

Sources: https://opencode.ai/docs/mcp-servers/ , /config/ , /cli/ , /server/ , /go/ , /zen/

### MCP config [V]
`opencode.json` (JSONC ok), top-level `mcp` object:
```jsonc
{ "$schema": "https://opencode.ai/config.json",
  "mcp": { "aihub": { "type": "remote", "url": "http://127.0.0.1:54321/mcp",
           "enabled": true, "oauth": false,
           "headers": { "Authorization": "Bearer {env:AIHUB_MCP_TOKEN}" } } },
  "tools": { "bash": false, "edit": false, "write": false },
  "permission": { "bash": "deny", "edit": "deny" } }
```
- remote: `type,url,enabled,headers,oauth(obj|false),timeout(ms, default 5000 tool discovery)`. local: `type:"local", command:[...], cwd, environment, enabled, timeout`. `{env:VAR}` substitution works in config (unset -> empty string); also `{file:path}` [U]. `oauth:false` disables auto OAuth detection (needed for static-header servers). MCP tool names are prefixed `<server>_<tool>`; disable via `tools: {"name*": false}` globs. `permission` accepts ask/allow/deny per tool [managed example uses "deny"; general doc says ask/allow - U]; `tools: {"bash": false}` is documented.
- Note: raise `timeout` above 5000 if our server is slow to list tools.

### Env/flags for custom config [V]
Precedence low->high: remote `.well-known/opencode` < global `~/.config/opencode/opencode.json` < `OPENCODE_CONFIG` (path to file) < project `opencode.json` < `.opencode` dirs < `OPENCODE_CONFIG_CONTENT` (inline JSON) < managed < macOS MDM. Merged, not replaced. Best for us: `OPENCODE_CONFIG_CONTENT='{...json...}'` env (no file, highest user-level precedence) or `OPENCODE_CONFIG=<temp file>`; `OPENCODE_CONFIG_DIR` for agents/commands/plugins/skills dir; `OPENCODE_PERMISSION` inline JSON permissions.
### Headless [V]
- `opencode run "prompt" [--format json] [-m provider/model] [--agent x] [-c|-s <id>] [--fork] [-f file] [--attach http://localhost:4096]`. `--format json` = raw JSON events. `--attach` reuses a running server (avoids MCP cold boot).
- `opencode serve [--port N] [--hostname 127.0.0.1]` (default 127.0.0.1:4096). Basic auth: `OPENCODE_SERVER_PASSWORD` (+ `OPENCODE_SERVER_USERNAME`, default `opencode`). API: `GET /doc` (OpenAPI 3.1), `POST /session`, `POST /session/:id/message` (waits), `POST /session/:id/prompt_async` (204), `GET /event` (SSE; first `server.connected`). Also `opencode web`. Use random port + password per launch, like our MCP server.
- Providers' keys: `opencode auth login|list|logout` -> `~/.local/share/opencode/auth.json`.

### OpenCode Zen [V]
Pay-as-you-go curated AI gateway; key from https://opencode.ai/auth (add billing, copy key); `Authorization: Bearer $OPENCODE_API_KEY`. Base `https://opencode.ai/zen/v1`; per-model endpoint: `/responses` (OpenAI Responses; gpt-5.x, grok, muse-spark), `/messages` (Anthropic format; claude-*, some qwen), `/models/<id>` (Gemini format), `/chat/completions` (OpenAI-compatible: kimi-k2.x/k3, glm-5.x, deepseek-v4*, minimax, qwen3.8-max, mistral-large-4, free models). Model list: `GET https://opencode.ai/zen/v1/models`. In opencode config ids look like `opencode/<model-id>`. So NOT uniformly OpenAI-compatible - route by model. Example ids seen today: gpt-5.5, claude-sonnet-5-5, claude-opus-5-5, gemini-3.5-flash, kimi-k3, glm-5.3, deepseek-v4-pro (list changes constantly; fetch /models at runtime).
### OpenCode Go [V]
Low-cost subscription to open coding models: Go $10/mo, Go Plus $40/mo; limits per model (5-hour 20%, weekly 50%, monthly 100% of a dollar cap; Zen balance fallback optional). Base `https://opencode.ai/zen/go/v1/` (models at `.../models`), same three paths `/responses`, `/messages`, `/chat/completions` chosen per model; ids `opencode-go/<id>` e.g. glm-5.3, kimi-k3, kimi-k2.7-code, deepseek-v4-pro, deepseek-v4-flash, minimax-m3, qwen3.8-max, mimo-v2.6-pro. Key: sign in to OpenCode Console, subscribe, copy key. Only one member per workspace can subscribe. Docs do not state whether non-OpenCode clients are permitted on Go keys [U - check ToS before offering Go in the built-in agent; Zen docs show direct HTTP use with Bearer, so Zen is safe].

---------------------------------------------------------------------------
## 4. DeepSeek API [V] (https://api-docs.deepseek.com/ , /quick_start/pricing , /guides/anthropic_api)

- OpenAI-format base URL: `https://api.deepseek.com` (path `/chat/completions`; docs also list a Responses API as supported). Anthropic-format base URL: `https://api.deepseek.com/anthropic`. Key: https://platform.deepseek.com/api_keys. Bearer auth.
- Current model ids (page dated now): `deepseek-flash` (= DeepSeek-V4.1-Flash; vision supported) and `deepseek-v4-pro` (= DeepSeek-V4-Pro-0813; no vision). Legacy `deepseek-v4-flash`, `deepseek-v4-flash-vision-exp` still accepted but served by V4.1-Flash. `deepseek-chat`/`deepseek-reasoner` aliases: not mentioned on the pricing page -> assume deprecated/removed [U]. Context 1M, max output 384K; thinking and non-thinking (thinking default; request `thinking:{type:"enabled"}`, `reasoning_effort`).
- Features table: tool calls yes (both), JSON output yes, Responses API yes, Anthropic API yes, chat-prefix (beta), FIM (beta, non-thinking only). Strict-mode tool calling uses a beta base URL (`/beta`) [U - guide page did not load; check /guides/function_calling]. In thinking mode with tool calls, reasoning_content must be echoed back per DeepSeek's docs [U].
- Anthropic endpoint: supports tools, tool_choice (most), system, stream, thinking, temperature; ignores cache_control, top_k, mcp_servers, anthropic-version; unsupported blocks: document, redacted_thinking, mcp_tool_use/result. Claude model names map: claude-opus* -> deepseek-v4-pro; haiku/sonnet/other -> deepseek-flash. Claude Code works against it via `ANTHROPIC_BASE_URL` + `ANTHROPIC_API_KEY` (possible alternative backend for the Claude Code integration, API-key billed to DeepSeek, no Anthropic subscription involved).

---------------------------------------------------------------------------
## 5. Versions (crates.io API + npm registry, queried 2026-10-10) [V]

| Crate | Latest stable | Notes |
|---|---|---|
| tauri | 2.12.2 | (3.0.0-alpha.4 exists - stay on 2.x) |
| tauri-build | 2.7.1 | |
| rmcp | 3.5.1 | official Rust MCP SDK (modelcontextprotocol/rust-sdk). Major v3 - API differs from 0.x tutorials. |
| russh | 0.64.1 | depends on `ssh-key =0.7.0-rc.11` internally; default features `flate2, aws-lc-rs, rsa`; alt `ring`. |
| russh-keys | 0.49.2 (last, Jan 2025) | **superseded/merged** into russh (`russh::keys`); don't add russh-keys. |
| ssh-key | 0.6.7 stable (0.7.0-rc.11 pinned by russh) | use `russh::keys` re-exports to avoid version mismatch. |
| portable-pty | 0.9.0 | (Feb 2025, WezTerm) ConPTY on Windows |
| argon2 | 0.6.0 | RustCrypto password-hash 0.6 era |
| chacha20poly1305 | 0.11.0 | `XChaCha20Poly1305` |
| zeroize | 1.9.1 | |
| keyring | 4.2.0 | v4 restructure, see below |
| tokio | 1.53.2 | |
| reqwest | 0.13.5 | |
| axum (rmcp example uses) | 0.8.9 | |

npm: `@tauri-apps/api` 2.12.2, `@tauri-apps/cli` 2.12.1, `svelte` 5.57.2, `@sveltejs/vite-plugin-svelte` 7.3.1, `vite` 8.3.4, `@xterm/xterm` 6.0.0, `@xterm/addon-webgl` 0.19.0, `@xterm/addon-fit` 0.11.0 (`@xterm/addon-web-links` 0.12.0). Check vite-plugin-svelte 7.x peer range vs vite 8 / svelte 5 when installing.

### rmcp 3.5.1 API notes [V features from crates.io; code from repo main README/example - re-check against 3.5.1 docs.rs]
- Default features: `base64, macros, server`. Server HTTP: enable `transport-streamable-http-server` (+ `server`, `macros`, `schemars` for `#[tool]` schemas); `transport-streamable-http-server-session` (session manager) and `server-side-http` also exist as features; `reqwest`(rustls) only needed for client. `transport-io` = stdio server; `auth` = OAuth. Cargo: `rmcp = { version = "3.5", features = ["server","macros","schemars","transport-streamable-http-server"] }` (add `-session` feature if the compiler asks for `LocalSessionManager`).
- Axum integration (from official example `counter_streamhttp.rs`):
```rust
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager};
let ct = tokio_util::sync::CancellationToken::new();
let svc = StreamableHttpService::new(|| Ok(MyServer::new()), LocalSessionManager::default().into(),
    StreamableHttpServerConfig::default().with_cancellation_token(ct.child_token()));
let app = axum::Router::new().nest_service("/mcp", svc)
    .layer(axum::middleware::from_fn_with_state(token, bearer_check)); // our own tower/axum auth layer
let l = tokio::net::TcpListener::bind("127.0.0.1:0").await?;   // random port
axum::serve(l, app).with_graceful_shutdown(async move { ct.cancelled().await }).await?;
```
  rmcp has no built-in bearer check for servers: wrap with axum middleware (constant-time compare of `Authorization: Bearer`), also validate `Origin`/`Host` for DNS-rebinding defense. Server struct uses `#[tool_router]` / `#[tool]` / `#[tool_handler]` macros with `Parameters<T: JsonSchema>`.
- README mentions MCP spec 2026-07-28 with stateless Streamable HTTP (no sessions/GET/DELETE) and MRTR. Older agent CLIs may negotiate older versions (sessions + SSE GET); keep session manager enabled and test each CLI (Claude Code, Codex, OpenCode) against it [U].

### Other API notes
- tauri 2: `tauri::Builder`, commands via `#[tauri::command]`, events/channels (`tauri::ipc::Channel<T>`) best for PTY output streaming to xterm.js; capabilities JSON in `src-tauri/capabilities/`. `tauri-build` in build.rs.
- russh 0.64: `client::connect(config, addr, handler)`, `Handler::check_server_key` (implement known_hosts check), `channel_open_session`, `request_pty`, `request_shell`, `channel.data()`, `window_change`; auth: `authenticate_password`, `authenticate_publickey(user, PrivateKeyWithHashAlg)`, agent support via `russh::keys::agent`. `russh::keys::load_secret_key`, `decode_secret_key`. Exact signatures changed across 0.4x-0.64 [U - confirm on docs.rs].
- portable-pty 0.9: `native_pty_system().openpty(PtySize{..})`, `slave.spawn_command(CommandBuilder)`, `master.try_clone_reader()`, `take_writer()`, `master.resize()`. Reader is blocking -> use `spawn_blocking`/thread. On Windows uses ConPTY (set env/args carefully; ConPTY mangles some escape sequences, may swallow OSC 133 on older Windows - test).
- argon2 0.6: `Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::new(m_kib, t, p, Some(32))?)`, `hash_password_into(pw, salt, &mut key)` for KDF use (no PHC string needed). OWASP-ish: m=64 MiB+, t=3, p=1-4.
- chacha20poly1305 0.11: `XChaCha20Poly1305::new(&key)`, `cipher.encrypt(&nonce24, Payload{msg,aad})`; random 24-byte nonce per message is safe; store `version|salt|nonce|ct`. 0.11 moved to new `aead` 0.6/crypto-common generics - API names `KeyInit`, `Aead`, `generate_nonce(&mut OsRng)` may have shifted [U].
- zeroize 1.9: derive `Zeroize, ZeroizeOnDrop`; wrap with `Zeroizing<Vec<u8>>` for keys/passwords.
- keyring 4.x: default feature `v1` keeps the old API (`keyring::Entry::new(service,user)?.set_password/get_password/delete_credential`) with native stores (Windows Credential Manager, macOS Keychain, Secret Service). v4 core split into `keyring-core` + per-store crates; for a stable simple setup use default `v1`. Windows credential blob limit ~2.5 KB (store only a 32-byte master/wrapping key, not blobs).
- reqwest 0.13: features `json,stream,rustls` (check default TLS backend changed to rustls in 0.13 [U]); `bytes_stream()` for SSE LLM streaming.

---------------------------------------------------------------------------
## 6. OSC 133 shell integration (FinalTerm/iTerm2/VS Code convention) [from memory + VS Code convention; snippets untested]

Sequence: `ESC ] 133 ; A ST` prompt start; `B` prompt end / command input start; `C` command output start (after Enter, before run); `D;<exit>` command finished. ST = `BEL (\a)` or `ESC \`. VS Code additionally uses `OSC 633` (adds `E;<cmdline>` and `P;Cwd=`); plain 133 is enough. xterm.js: register with `term.parser.registerOscHandler(133, data => ...)`, data = `A`, `B`, `C`, `D;0`.

### bash (>= 4.4 for PS0)
```bash
__aih_ran=
__aih_pc() { local ec=$?; [[ -n $__aih_ran ]] && printf '\e]133;D;%s\a' "$ec"; __aih_ran=1; }
case ";$PROMPT_COMMAND;" in *";__aih_pc;"*) ;; *) PROMPT_COMMAND="__aih_pc${PROMPT_COMMAND:+;$PROMPT_COMMAND}";; esac
PS1='\[\e]133;A\a\]'"${PS1}"'\[\e]133;B\a\]'
PS0='\e]133;C\a'          # printed after the line is read, before execution (bash 4.4+)
```
Notes: keep `__aih_pc` first in PROMPT_COMMAND so `$?` is the command's status. Older bash: use `trap '...' DEBUG` as preexec (guard with a `__aih_in_prompt` flag so it fires once per command, not for each PROMPT_COMMAND sub-command) or use `bash-preexec`. PS0 expands `\e`/`\a` like PS1 (no `\[ \]` needed since it is not part of the line editor width). Guard against double-wrapping PS1 when rc is re-sourced.

### zsh
```zsh
autoload -Uz add-zsh-hook
__aih_precmd()  { local ec=$?; [[ -n $__aih_ran ]] && print -n "\e]133;D;${ec}\a"; print -n "\e]133;A\a"; }
__aih_preexec() { __aih_ran=1; print -n "\e]133;C\a"; }
precmd_functions=(__aih_precmd ${precmd_functions:#__aih_precmd})   # run first to see real $?
add-zsh-hook preexec __aih_preexec
[[ $PS1 == *133\;B* ]] || PS1="${PS1}%{"$'\e]133;B\a'"%}"
```
Notes: B is appended inside `%{ %}` (zero-width). Prompt themes that rebuild PS1 every precmd (p10k, pure) will drop B: re-append inside `__aih_precmd` after themes ran (put hook last for that part) - p10k has `POWERLEVEL9K_TERM_SHELL_INTEGRATION=true` for 133.

### fish
```fish
function __aih_prompt_start --on-event fish_prompt
    printf '\e]133;A\a'
end
function __aih_preexec --on-event fish_preexec
    printf '\e]133;C\a'
end
function __aih_postexec --on-event fish_postexec
    printf '\e]133;D;%s\a' $status
end
if not functions -q __aih_orig_prompt
    functions -c fish_prompt __aih_orig_prompt
    function fish_prompt
        __aih_orig_prompt
        printf '\e]133;B\a'
    end
end
```
Notes: fish 4.x emits OSC 133 natively [U - verify version/which marks]; detect with `fish --version` and skip our snippet to avoid duplicates. `$status` inside `fish_postexec` handler = last command status (WezTerm uses same pattern) [U].

Delivery to remote hosts (SSH): don't edit user rc silently. Options: (1) opt-in "install integration" that appends a guarded `[ -n "$AIHUB_TERM" ] && source ~/.aihub_shellint.sh` block; (2) after login, inject the snippet by sending a one-line `eval "$(printf ...)"` with leading space/`HISTCONTROL=ignorespace` (noisy, echoes); (3) fallback with no integration: marker-based detection (inject `printf '\e]133;...'` sentinels around agent-issued commands in `ssh_exec` tool: `cmd; printf '\e]133;D;%s\a' $?`). For agent-driven commands, sentinel wrapping with a random nonce is more robust than relying on prompts.

---------------------------------------------------------------------------
## 7. Import formats (brief; all [U] - from memory/knowledge, verify with real sample files)

### MobaXterm (`MobaXterm.ini` / exported `.mxtsessions`)
INI-style. Sessions live in `[Bookmarks]`, `[Bookmarks_1]`, `[Bookmarks_2]`... Each section: `SubRep=<folder path with \>`, `ImgNum=<icon>`, then `<SessionName>=<value>` lines. SSH value: `#109#0%host%port%username%...` i.e. `#<typeId>#<n>%` then `%`-separated fields: [1]=host, [2]=port, [3]=user (empty = prompt), later fields include "use private key" flag and key path (`%-1%-1%%%%%0%0%0%<keyfile path>%...` position varies by version), then a `#MobaFont%...` terminal-settings block and trailing `#<n>#<comment>`. Type ids seen: 109=SSH, 91=RDP, 98=Telnet?, 100=VNC? [U beyond 109]. Passwords are NOT in the session line; stored in `[Passwords]` (`user@host=`) / `[Credentials]` encrypted with master password (AES, key from master password) - skip, import hosts only. `.mxtsessions` = same text format (zip not required).

### mRemoteNG `confCons.xml`
Root `<Connections Name="Connections" Export="false" EncryptionEngine="AES" BlockCipherMode="GCM" KdfIterations="1000" FullFileEncryption="false" Protected="..." ConfVersion="2.6">`; each `<Node Name="" Type="Connection|Container" Descr="" Icon="" Panel="General" Id="GUID" Username="" Domain="" Password="<base64>" Hostname="" Protocol="SSH2|SSH1|RDP|Telnet|VNC..." Port="22" PuttySession="Default Settings" ...>` nested for folders (`Type="Container"` with child Nodes). Password attribute: base64 of AES-GCM blob; key = PBKDF2(HMAC-SHA1, password, salt, KdfIterations) -> 256-bit; layout: salt (16 B) || nonce (16 B?) || ciphertext || tag (16 B), salt reused as AAD [U on exact offsets: read mRemoteNG `AeadCryptographyProvider.cs`]. Default password when no master password: **`mR3m`**. `Protected` attr = encrypted check string used to verify the master password. Older files (ConfVersion < 2.6): AES-CBC, IV = first 16 B, key = MD5(password). `FullFileEncryption="true"` -> whole file body is encrypted blob.

### PuTTY
Windows registry `HKCU\Software\SimonTatham\PuTTY\Sessions\<name>` (name URL-escaped, e.g. `My%20Server`; `Default%20Settings` is template). Values: `HostName` (REG_SZ; may hold `user@host`), `PortNumber` (DWORD), `Protocol` ("ssh"), `UserName`, `PublicKeyFile` (path .ppk), `AgentFwd`, `Proxy*`, `LocalPortForwardings`/`RemotePortForwardings` (strings like `L8080=host:80,`), `Compression`, `TerminalType`, `Font*`/colours. No passwords stored. Host keys: `HKCU\Software\SimonTatham\PuTTY\SshHostKeys` (`ssh-ed25519@22:host` = key). Unix: `~/.putty/sessions/<name>` key=value files. Export via `reg export`/ `.reg` file parse.

### WinSCP
Config: registry `HKCU\Software\Martin Prikryl\WinSCP 2\Sessions\<name>` or `WinSCP.ini` (portable / `%APPDATA%\WinSCP.ini`) sections `[Sessions\<url-escaped name>]` (folders via `/` in name, e.g. `Folder/Server`). Keys: `HostName`, `PortNumber`, `UserName`, `Password` (hex string), `PublicKeyFile`, `FSProtocol` (0=SCP, 1=SFTP+SCP?, 2=SFTP, 5=FTP, 6=WebDAV, 7=S3 - meaning varies [U]), `ProxyMethod`..., `Tunnel*` (SSH tunnel). Password obfuscation (not encryption; not available if a master password is set): hex nibble pairs; decode each byte `b = ~(((hi<<4)|lo) ^ 0xA3) & 0xFF` where 0xA3 = PWALG_SIMPLE_MAGIC. Stream: read byte `flag`; if `flag == 0xFF` -> skip one byte, then `length = next byte`; else `length = flag`; then `skip = next byte` -> discard `skip` bytes; then decode `length` bytes. Plaintext = `username + hostname + password`; strip the `username+hostname` prefix (if it doesn't match -> corrupt/other key). Because it is obfuscation, treat as credential import requiring explicit user consent.

### SecureCRT
Sessions stored as `.ini` files under `Config\Sessions\<folder>\<name>.ini` (lines `S:"Hostname"=host`, `D:"[SSH2] Port"=00000016` (hex dword), `S:"Username"=user`, `S:"Protocol Name"=SSH2`, `S:"Identity Filename V2"=path`, `S:"Password V2"=02:<hex>` encrypted), or XML export (File > Export): `<VanDyke version="3.0"><key name="Sessions"><key name="Folder"><key name="MyHost"><string name="Hostname">h</string><dword name="[SSH2] Port">22</dword><string name="Username">u</string><string name="Protocol Name">SSH2</string>...`. Passwords: "Password V2" prefix `02:` (AES-256-CBC with key derived from a configuration passphrase, default empty -> decryptable) or `03:` (bcrypt_kdf-based) - do not support; import host/user/port/key path only.

### Tabby `config.yaml`
Location `%APPDATA%\tabby\config.yaml` (macOS `~/Library/Application Support/tabby/`, Linux `~/.config/tabby/`). Top-level `profiles:` list; SSH profile:
```yaml
profiles:
  - id: ssh:custom:my-server:<uuid>
    type: ssh
    name: my-server
    group: <groupId>            # references groups: [{id,name}]
    icon: ...
    options:
      host: 1.2.3.4
      port: 22
      user: root
      auth: publicKey           # password | publicKey | agent | keyboardInteractive
      privateKeys: [ "C:/Users/me/.ssh/id_ed25519" ]
      jumpHost: <profileId>
      algorithms: {...}
      forwardedPorts: [{type: Local, host, port, targetAddress, targetPort}]
      scripts: [{expect, send}]
    behaviorOnSessionEnd: auto
```
Passwords/passphrases are not in config.yaml; they are in the encrypted Tabby vault (`vault:` block with `contents` AES) when enabled - skip. `groups:` for folders; `connectionGroups` in older versions.

---------------------------------------------------------------------------
## Open items to verify before coding
1. Claude `--input-format stream-json` exact message schema (run `claude -p --input-format stream-json --output-format stream-json --verbose --replay-user-messages` and inspect).
2. Codex `-c mcp_servers.x.*` overrides and `features.shell_tool=false` behavior on the installed version; `mcp_tool_call` item shape.
3. rmcp 3.5.1 exact types (docs.rs) and which protocol version each agent CLI negotiates against the Streamable HTTP server.
4. Import formats: collect real sample files (MobaXterm field positions, mRemoteNG GCM layout, SecureCRT password versions).
5. Written confirmation from Anthropic (and OpenAI) on third-party apps launching users' CLIs under subscription login.
