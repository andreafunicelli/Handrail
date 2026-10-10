// Mirrors crates/aht-core (serde snake_case; tagged enums use a `type` field; ids are UUID strings).

export type Uuid = string;
export type ConnectionId = Uuid;
export type GroupId = Uuid;
export type SecretId = Uuid;
export type ProviderId = Uuid;
export type HarnessProfileId = Uuid;
export type SessionId = Uuid;
export type OutputId = Uuid;
export type ApprovalId = Uuid;

// ---- model.rs ----
export type ConnectionKind = "ssh" | "mysql" | "sftp" | "local";
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
export type HarnessDisplay = "sidebar" | "native";

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
  source_id: string;
  tool: string;
  path: string;
  count: number;
}

/** `import_preview` result. The parsed bundle (incl. secrets) stays server-side under `preview_id`. */
export interface ImportPreview {
  preview_id: string;
  tool: string;
  groups: Group[];
  /** Connections without secret values. */
  connections: Connection[];
  warnings: string[];
  /** `index` into `connections` that duplicates an existing connection (host+port+user). */
  duplicates: { index: number; existing_id: ConnectionId }[];
}

/** Contract addition: shape of `sync_status()`. */
export interface SyncStatus {
  backend: "none" | "github" | "folder";
  target: string | null;
  last_sync: string | null;
  error: string | null;
}

/** Contract addition: global approval policy. */
export interface ApprovalSettings {
  default_mode: ApprovalMode;
  deny_list: string[];
}

export interface ProviderTestResult {
  ok: boolean;
  message: string | null;
}
