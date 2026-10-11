import type {
  FileEntry,
  TransferProgress,
  Snippet,
  S3Account,
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
  AuthPromptEvent,
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
  ConflictView,
  FolderPreset,
  SnapshotInfo,
  UpdateInfo,
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
  "session://auth_prompt": AuthPromptEvent;
  "transfer://progress": TransferProgress;
  "approval://requested": ApprovalRequest;
  "agent://event": AgentEventEnvelope;
  "sync://status": SyncStatus;
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
  /** Saves the Emergency Kit text to the Downloads folder; returns the file path. */
  saveEmergencyKit(contents: string): Promise<string>;

  // connections & groups
  listConnections(): Promise<Connection[]>;
  saveConnection(connection: Connection): Promise<Connection>;
  deleteConnection(id: string): Promise<void>;
  listGroups(): Promise<Group[]>;
  saveGroup(group: Group): Promise<Group>;
  deleteGroup(id: string): Promise<void>;
  saveSecret(secret: Secret): Promise<SecretId>;

  // sessions
  /** `promptRef` lets a connection without saved credentials ask for them while logging in
   *  ("session://auth_prompt" events carry it back). */
  openSession(connectionId: string, cols: number, rows: number, promptRef?: string): Promise<SessionInfo>;
  /** Answer a login prompt; `null` cancels the login. */
  answerAuthPrompt(requestId: string, answers: string[] | null): Promise<void>;
  openLocalSession(shell: string | null, cols: number, rows: number): Promise<SessionInfo>;
  sessionWrite(sessionId: string, dataB64: string): Promise<void>;
  sessionResize(sessionId: string, cols: number, rows: number): Promise<void>;
  closeSession(sessionId: string): Promise<void>;
  hostKeyDecision(requestId: string, accept: boolean): Promise<void>;

  // approvals
  approvalDecide(approvalId: string, decision: ApprovalDecision): Promise<void>;
  setApprovalMode(sessionId: string, mode: ApprovalMode): Promise<void>;
  getApprovalSettings(): Promise<ApprovalSettings>;
  saveApprovalSettings(settings: ApprovalSettings): Promise<void>;

  // providers / harnesses / agent
  // files (SFTP on an SSH session's connection)
  sftpHome(sessionId: string): Promise<string>;
  sftpList(sessionId: string, path: string): Promise<FileEntry[]>;
  sftpMkdir(sessionId: string, path: string): Promise<void>;
  sftpRename(sessionId: string, from: string, to: string): Promise<void>;
  sftpRemove(sessionId: string, path: string, isDir: boolean): Promise<void>;
  /** Download into the Downloads folder; resolves with the local path. Progress: "transfer://progress". */
  sftpDownload(sessionId: string, path: string, transferId: string): Promise<string>;
  /** Upload a local file into `dir`; resolves with the remote path. */
  sftpUpload(sessionId: string, localPath: string, dir: string, transferId: string): Promise<string>;
  /** Native file picker; empty when cancelled. */
  pickFiles(): Promise<string[]>;

  listS3Accounts(): Promise<S3Account[]>;
  /** `secretKey` replaces the stored secret access key when given. */
  saveS3Account(account: S3Account, secretKey: string | null): Promise<S3Account>;
  deleteS3Account(id: string): Promise<void>;
  /** S3 paths are "/bucket/folder/file"; "/" lists the buckets. */
  s3Home(accountId: string): Promise<string>;
  s3List(accountId: string, path: string): Promise<FileEntry[]>;
  s3Mkdir(accountId: string, path: string): Promise<void>;
  /** Files only: S3 folders are key prefixes. */
  s3Rename(accountId: string, from: string, to: string, isDir: boolean): Promise<void>;
  s3Remove(accountId: string, path: string, isDir: boolean): Promise<void>;
  s3Download(accountId: string, path: string, transferId: string): Promise<string>;
  s3Upload(accountId: string, localPath: string, dir: string, transferId: string): Promise<string>;

  listSnippets(): Promise<Snippet[]>;
  saveSnippet(snippet: Snippet): Promise<Snippet>;
  deleteSnippet(id: string): Promise<void>;
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
  syncSetAuto(enabled: boolean): Promise<void>;
  syncDisconnect(): Promise<void>;
  syncFolderPresets(): Promise<FolderPreset[]>;

  // snapshots & conflict history
  vaultSnapshots(): Promise<SnapshotInfo[]>;
  vaultSnapshotCreate(): Promise<SnapshotInfo>;
  vaultSnapshotRestore(name: string): Promise<void>;
  vaultConflicts(): Promise<ConflictView[]>;
  vaultConflictRestore(fileId: string, recordedAtMs: number): Promise<void>;
  vaultConflictsClear(): Promise<void>;

  // app & updates
  appVersion(): Promise<string>;
  /** Asks the release endpoint for a newer version; `null` when up to date. */
  checkForUpdate(): Promise<UpdateInfo | null>;
  /** Downloads and installs the update found by the last check. `progress` gets 0..1, or null
   *  while the size is unknown. On Windows the installer then closes and restarts the app. */
  installUpdate(progress: (fraction: number | null) => void): Promise<void>;
  restartApp(): Promise<void>;
  /** Opens an https link in the default browser (the webview ignores target="_blank"). */
  openUrl(url: string): Promise<void>;

  // misc

  on<E extends EventName>(event: E, cb: (payload: EventMap[E]) => void): Promise<Unlisten>;
}
