import type {
  AgentEventEnvelope,
  AgentMarkEvent,
  ApprovalDecision,
  ApprovalMode,
  ApprovalRequest,
  ApprovalSettings,
  Brain,
  Connection,
  DetectedHarness,
  Group,
  HarnessExtensions,
  HarnessKind,
  HarnessProfile,
  HostKeyEvent,
  GithubDeviceStart,
  GithubPoll,
  ImportPreview,
  ImportPreviewArgs,
  ImportSource,
  Provider,
  ProviderTestResult,
  Secret,
  SecretId,
  SessionDataEvent,
  SessionInfo,
  SyncReport,
  SyncStatus,
  VaultStatus,
} from "./types";

export type Unlisten = () => void;

export interface EventMap {
  "session://data": SessionDataEvent;
  "session://state": SessionInfo;
  "session://agent_mark": AgentMarkEvent;
  "session://host_key": HostKeyEvent;
  "approval://requested": ApprovalRequest;
  "agent://event": AgentEventEnvelope;
}
export type EventName = keyof EventMap;

/** Typed surface of docs/ipc-contract.md. `tauri.ts` and `mock.ts` both implement it. */
export interface Api {
  readonly backend: "tauri" | "mock";

  // vault
  vaultStatus(): Promise<VaultStatus>;
  vaultCreate(password: string): Promise<{ secret_key: string }>;
  vaultUnlock(password: string, secretKey?: string): Promise<void>;
  vaultLock(): Promise<void>;

  // connections & groups
  listConnections(): Promise<Connection[]>;
  saveConnection(connection: Connection): Promise<Connection>;
  deleteConnection(id: string): Promise<void>;
  listGroups(): Promise<Group[]>;
  saveGroup(group: Group): Promise<Group>;
  deleteGroup(id: string): Promise<void>;
  saveSecret(secret: Secret): Promise<SecretId>;

  // sessions
  openSession(connectionId: string, cols: number, rows: number): Promise<SessionInfo>;
  openLocalSession(shell: string | null, cols: number, rows: number): Promise<SessionInfo>;
  sessionWrite(sessionId: string, dataB64: string): Promise<void>;
  sessionResize(sessionId: string, cols: number, rows: number): Promise<void>;
  closeSession(sessionId: string): Promise<void>;
  listSessions(): Promise<SessionInfo[]>;
  hostKeyDecision(requestId: string, accept: boolean): Promise<void>;

  // approvals
  approvalDecide(approvalId: string, decision: ApprovalDecision): Promise<void>;
  setApprovalMode(sessionId: string, mode: ApprovalMode): Promise<void>;
  getApprovalSettings(): Promise<ApprovalSettings>;
  saveApprovalSettings(settings: ApprovalSettings): Promise<void>;

  // providers / harnesses / agent
  listProviders(): Promise<Provider[]>;
  saveProvider(provider: Provider, apiKey?: string): Promise<Provider>;
  deleteProvider(id: string): Promise<void>;
  listModels(providerId: string): Promise<string[]>;
  testProvider(providerId: string): Promise<ProviderTestResult>;
  detectHarnesses(): Promise<DetectedHarness[]>;
  listHarnessProfiles(): Promise<HarnessProfile[]>;
  saveHarnessProfile(profile: HarnessProfile): Promise<HarnessProfile>;
  deleteHarnessProfile(id: string): Promise<void>;
  listHarnessExtensions(kind: HarnessKind): Promise<HarnessExtensions>;
  agentSend(sessionId: string, brain: Brain, message: string): Promise<{ run_id: string }>;
  agentCancel(runId: string): Promise<void>;
  harnessOpenNative(sessionId: string, profileId: string, cols: number, rows: number): Promise<SessionInfo>;
  harnessLogin(kind: HarnessKind, cols: number, rows: number): Promise<SessionInfo>;

  // import & sync
  importDetect(): Promise<ImportSource[]>;
  importPreview(args: ImportPreviewArgs): Promise<ImportPreview>;
  importApply(previewId: string, skipIndices: number[]): Promise<{ imported: number }>;
  syncStatus(): Promise<SyncStatus>;
  syncGithubStart(): Promise<GithubDeviceStart>;
  syncGithubPoll(): Promise<GithubPoll>;
  syncConfigureFolder(path: string): Promise<void>;
  syncJoin(): Promise<void>;
  syncNow(): Promise<SyncReport>;

  // misc
  getSetting<T = unknown>(key: string): Promise<T | null>;
  setSetting(key: string, value: unknown): Promise<void>;

  on<E extends EventName>(event: E, cb: (payload: EventMap[E]) => void): Promise<Unlisten>;
}
