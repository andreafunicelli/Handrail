// Mirrors crates/aht-core (serde snake_case; tagged enums use a `type` field; ids are UUID strings).

export type Uuid = string;
export type ConnectionId = Uuid;
export type GroupId = Uuid;
export type SecretId = Uuid;
export type IdentityId = Uuid;
export type ProviderId = Uuid;
export type HarnessProfileId = Uuid;
export type SessionId = Uuid;
export type OutputId = Uuid;
export type ApprovalId = Uuid;

// ---- model.rs ----
export type ConnectionKind = "ssh" | "mysql" | "sftp" | "local" | "rdp";
export type Environment = "prod" | "staging" | "dev" | "none";

export type SshAuth =
  | { type: "password"; secret: SecretId }
  | { type: "key"; key: SecretId; passphrase: SecretId | null }
  | { type: "agent" }
  | { type: "prompt" };

export interface Connection {
  id: ConnectionId;
  name: string;
  kind: ConnectionKind;
  group_id: GroupId | null;
  host: string;
  port: number;
  user: string;
  auth: SshAuth;
  jump: ConnectionId[];
  env: Environment;
  tags: string[];
  color: string | null;
  approval_mode: ApprovalMode | null;
  local_shell: string | null;
  notes: string;
  /** Port forwards started with every session of this connection. */
  tunnels: Tunnel[];
  /** RDP: connect to the console (administrative) session, like `mstsc /admin`. */
  rdp_admin?: boolean;
  /** Shown before the name: a built-in icon name or a single emoji. */
  icon?: string | null;
  /** Log in as this identity: its user and authentication replace `user` and `auth` when
   *  connecting, so changing the identity changes every connection using it. */
  identity?: IdentityId | null;
}

/** A reusable login: a user name with its password, key or the SSH agent. */
export interface Identity {
  id: IdentityId;
  label: string;
  user: string;
  /** Never `prompt`. */
  auth: SshAuth;
}

/** local: `ssh -L` (listen here, connect from the server); remote: `ssh -R` (listen on the
 *  server, connect from here); dynamic: `ssh -D` (a SOCKS5 proxy here). */
export type TunnelKind = "local" | "remote" | "dynamic";

export interface Tunnel {
  kind: TunnelKind;
  bind_host: string;
  bind_port: number;
  /** Unused by dynamic forwards. */
  dest_host: string;
  dest_port: number;
  enabled: boolean;
}

/** A session's tunnel: running on `port`, failed with `error`, or (neither) still starting. */
export interface TunnelStatus {
  tunnel: Tunnel;
  port: number | null;
  error: string | null;
}

export interface TunnelsEvent {
  session_id: string;
  tunnels: TunnelStatus[];
}

/** A Linux server's load, read over the session's connection. Missing values are null
 *  (another OS, or a /proc file that could not be read). */
export interface ServerStats {
  /** Percent of all cores, over half a second. */
  cpu: number | null;
  cores: number | null;
  mem_used: number | null;
  mem_total: number | null;
  /** The root file system. */
  disk_used: number | null;
  disk_total: number | null;
  load1: number | null;
  uptime_s: number | null;
}

export interface Group {
  id: GroupId;
  name: string;
  parent_id: GroupId | null;
  order: number;
}

export type SecretKind = "password" | "private_key" | "passphrase" | "api_key" | "token";

export interface Secret {
  id: SecretId;
  kind: SecretKind;
  label: string;
  value: string;
}

/** A saved secret without its value. */
export type SecretInfo = Omit<Secret, "value">;

export type ApprovalMode = "confirm_all" | "confirm_writes" | "auto";

export type ProviderKind = "anthropic" | "open_ai" | "open_ai_compat";
export type ProviderPreset =
  | "anthropic"
  | "open_ai"
  | "deep_seek"
  | "open_code_go"
  | "open_code_zen"
  | "open_router"
  | "ollama"
  | "lm_studio"
  | "custom";

export interface Provider {
  id: ProviderId;
  name: string;
  kind: ProviderKind;
  preset: ProviderPreset;
  base_url: string;
  api_key: SecretId | null;
  default_model: string;
}

export type HarnessKind = "claude_code" | "codex" | "open_code";
/** `open_chamber`: OpenCode only, the OpenChamber web UI in a window of its own. */
export type HarnessDisplay = "sidebar" | "native" | "open_chamber";

export interface HarnessProfile {
  id: HarnessProfileId;
  name: string;
  harness: HarnessKind;
  display: HarnessDisplay;
  server_mode: boolean;
  skills: string[];
  plugins: string[];
  extra_args: string[];
  model: string | null;
}

export type Brain =
  | { type: "built_in"; provider: ProviderId; model: string }
  | { type: "harness"; profile: HarnessProfileId };

// ---- terminal.rs ----
export type IntegrationMode = "full" | "limited" | "unknown";
export type SessionState =
  | "connecting"
  | "idle"
  | "busy"
  | "agent_running"
  | "full_screen"
  | "reconnecting"
  | "disconnected";

export interface SessionInfo {
  session_id: SessionId;
  connection_id: ConnectionId | null;
  title: string;
  env: Environment;
  os: string | null;
  shell: string | null;
  cwd: string | null;
  user: string | null;
  host: string | null;
  integration: IntegrationMode;
  state: SessionState;
  approval_mode: ApprovalMode;
  /** Why the session ended, once it is "disconnected". */
  ended?: SessionEnd | null;
}

/** The shell exited (`exit_status`), or the connection was lost (`error`). */
export interface SessionEnd {
  exit_status: number | null;
  error: string | null;
}

export type CommandState = "completed" | "timed_out" | "interrupted" | "needs_user" | "denied";

export interface CommandResult {
  state: CommandState;
  exit_code: number | null;
  output: string;
  truncated: boolean;
  output_id: OutputId | null;
  duration_ms: number;
  command: string;
  note: string | null;
}

// ---- agent.rs ----
export type RiskClass = "read" | "write" | "destructive";

export interface ApprovalRequest {
  approval_id: ApprovalId;
  session_id: SessionId;
  actor_label: string;
  command: string;
  reason: string | null;
  risk: RiskClass;
  policy_note: string | null;
}

export type ApprovalDecision =
  | { type: "approve" }
  | { type: "approve_edited"; command: string }
  | { type: "deny"; message: string | null };

/** `ApprovalRequested(ApprovalRequest)` is a newtype variant: serde inlines the struct fields. */
export type AgentEvent =
  | { type: "text_delta"; text: string }
  | { type: "thinking_delta"; text: string }
  | { type: "tool_call"; call_id: string; name: string; input: unknown }
  | { type: "tool_result"; call_id: string; output: unknown; is_error: boolean }
  | ({ type: "approval_requested" } & ApprovalRequest)
  | { type: "approval_resolved"; approval_id: ApprovalId; decision: ApprovalDecision }
  | { type: "usage"; input_tokens: number; output_tokens: number; cost_usd: number | null }
  | { type: "error"; message: string; retryable: boolean }
  | { type: "step_limit"; steps: number }
  | { type: "done" };

// ---- IPC payloads (docs/ipc-contract.md) ----
export interface VaultStatus {
  initialized: boolean;
  locked: boolean;
  /** Contract addition: true when the vault exists but this device has no Secret Key stored yet. */
  needs_secret_key?: boolean;
}

export interface SessionDataEvent {
  session_id: SessionId;
  data_b64: string;
}
export interface AgentMarkEvent {
  session_id: SessionId;
  command: string;
  actor_label: string;
}
export interface HostKeyEvent {
  request_id: string;
  host: string;
  fingerprint: string;
  kind: "new" | "changed";
  /** Contract addition (optional): previously trusted fingerprint, for the changed-key warning. */
  old_fingerprint?: string;
  /** Contract addition (optional): key algorithm, e.g. "ssh-ed25519". */
  key_type?: string;
}
/** The server asks something while logging in (password, OTP) for a connection without saved
 *  credentials. Answer with `answerAuthPrompt`, one answer per prompt. */
/** One entry of a remote folder (SFTP). */
export interface FileEntry {
  name: string;
  path: string;
  /** For a symlink, its target's kind ("symlink" when the target is missing). */
  kind: "dir" | "file" | "symlink" | "other";
  /** A symbolic link: open follows it, delete removes only the link. */
  link: boolean;
  size: number;
  /** Seconds since the Unix epoch. */
  modified: number | null;
  /** Unix mode bits (type and permissions). */
  permissions: number | null;
}
/** What a file looked like when read; a save compares it to notice changes made meanwhile. */
export interface FileStamp {
  size: number;
  modified: number | null;
  /** SHA-256 of the contents: what the save's conflict check compares. */
  hash?: string | null;
}
export interface TextFile {
  text: string;
  stamp: FileStamp;
}
/** An agent's proposed new contents for a file, shown as a diff (event "edit://proposal"). */
export interface EditProposal {
  proposal_id: string;
  session_id: SessionId;
  path: string;
  /** Read through the root SFTP server; saving must use it too. */
  root: boolean;
  /** The SSH user may not read the file: reading it as root (sudo) shows the diff. */
  unreadable?: boolean;
  /** Current contents; null for a new file (or an unreadable one). */
  original: string | null;
  stamp: FileStamp | null;
  proposed: string;
  reason: string | null;
  actor_label: string;
}
export type EditOutcome = "saved" | "rejected" | "failed";
/** One end of a copy between file browsers. */
export type Place = { kind: "sftp"; session_id: string; root: boolean } | { kind: "s3"; account_id: string };

export interface TransferProgress {
  transfer_id: string;
  done: number;
  total: number;
}

/** S3 or S3-compatible storage credentials; the secret key stays in the backend. */
export interface S3Account {
  id: Uuid;
  name: string;
  /** Service URL for S3-compatible stores; empty for AWS. */
  endpoint: string;
  /** "auto" for Cloudflare R2. */
  region: string;
  /** Open this bucket directly instead of the bucket list. */
  bucket: string | null;
  access_key_id: string;
  /** Vault id of the stored secret access key. */
  secret: Uuid | null;
  path_style: boolean;
}

/** A saved command; `{{name}}` placeholders are asked for when it runs. */
export interface Snippet {
  id: Uuid;
  name: string;
  command: string;
  description: string;
  tags: string[];
}

export interface AuthPromptEvent {
  request_id: string;
  /** The `promptRef` passed to `openSession`. */
  prompt_ref: string;
  name: string;
  /** Shown above the prompts, e.g. "Permission denied, please try again." */
  instructions: string;
  prompts: { prompt: string; echo: boolean }[];
}
export interface AgentEventEnvelope {
  run_id: string;
  session_id: SessionId;
  event: AgentEvent;
}

export interface DetectedHarness {
  kind: HarnessKind;
  path?: string | null;
  version?: string | null;
  logged_in?: boolean | null;
}

export interface HarnessExtension {
  name: string;
  path: string;
}
export interface HarnessExtensions {
  skills: HarnessExtension[];
  plugins: HarnessExtension[];
}

export interface ImportSource {
  id: string;
  tool: string;
  path?: string | null;
  count_hint?: number | null;
}

export interface ImportPreviewArgs {
  source_id?: string;
  tool?: string;
  file_path?: string;
  mremoteng_password?: string;
  import_passwords?: boolean;
}

/** `import_preview` result. The parsed bundle (incl. secrets) stays server-side under `preview_id`. */
export interface ImportPreview {
  preview_id: string;
  tool: string;
  groups: Group[];
  /** Connections without secret values. */
  connections: Connection[];
  secret_count: number;
  warnings: string[];
  /** `index` into `connections` that duplicates an existing connection (host+port+user). */
  duplicates: { index: number; existing_id: ConnectionId }[];
}

export type SyncConfig = { kind: "github"; owner: string; repo: string; branch: string } | { kind: "folder"; path: string };

export interface SyncReport {
  pulled: number;
  pushed: number;
  conflicts: number;
  warnings: string[];
}

export interface SyncStatus {
  config?: SyncConfig | null;
  last_sync_ms?: number | null;
  last_report?: SyncReport | null;
  can_join: boolean;
  /** Background sync on unlock, after edits and every few minutes. */
  auto: boolean;
  /** A sync is running right now. */
  syncing: boolean;
  /** Error of the last attempt; cleared by the next successful sync. */
  last_error?: string | null;
}

/** A newer release found by the updater. */
export interface UpdateInfo {
  version: string;
  current_version: string;
  notes?: string | null;
  date?: string | null;
}

export type CloudDrive = "icloud" | "dropbox" | "onedrive" | "google_drive";

/** A cloud-drive folder found on this device (`sync_folder_presets`). */
export interface FolderPreset {
  drive: CloudDrive;
  label: string;
  /** `<drive>/Handrail` */
  path: string;
  /** Another device already syncs a vault there. */
  has_vault: boolean;
}

export interface SnapshotInfo {
  name: string;
  created_ms: number;
  size: number;
}

/** A version discarded by a last-writer-wins merge (payload stays in the backend). */
export interface ConflictView {
  file_id: string;
  recorded_at_ms: number;
  item_type: string;
  label: string;
  loser_ms: number;
  loser_device: string;
  loser_deleted: boolean;
  winner_ms: number;
}

export interface GithubDeviceStart {
  device_code: string;
  user_code: string;
  verification_uri: string;
  interval: number;
  expires_in: number;
}

export interface GithubPoll {
  done: boolean;
  status: "pending" | "slow_down" | "expired" | "denied" | "ok";
  repo?: string | null;
}

/** Backend errors reject with this shape (aht-core `Error`). */
export interface IpcError {
  kind: string;
  message: string;
}

/** Global approval policy, stored via `set_setting({ key: "approvals" })`. */
export interface ApprovalSettings {
  default_mode: ApprovalMode;
  deny_list: string[];
  /** Model calls the built-in agent may make per message (default 50). */
  max_steps?: number;
}

export interface ProviderTestResult {
  ok: boolean;
  message: string | null;
}

/** An RDP view's size in device pixels; `scale` in percent (100 = none). */
export interface RdpSize {
  width: number;
  height: number;
  scale: number;
}

/** One keyboard or mouse event for an RDP session (rdp-helper/src/proto.rs `Input`). */
export type RdpInput =
  | { t: "move"; x: number; y: number }
  | { t: "down" | "up"; button: number }
  | { t: "wheel"; vertical: boolean; units: number }
  | { t: "key"; code: number; down: boolean }
  | { t: "release_all" };
