import type {
  FileEntry,
  FileStamp,
  TextFile,
  EditProposal,
  EditOutcome,
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
  Identity,
  ImportPreview,
  ImportPreviewArgs,
  ImportSource,
  Provider,
  ProviderTestResult,
  Secret,
  SecretId,
  SecretInfo,
  SessionDataEvent,
  SessionInfo,
  Place,
  ServerStats,
  TunnelStatus,
  TunnelsEvent,
  ConflictView,
  FolderPreset,
  SnapshotInfo,
  UpdateInfo,
  SyncReport,
  SyncStatus,
  VaultStatus,
  RdpInput,
  RdpSize,
} from "./types";

export type Unlisten = () => void;

export interface EventMap {
  "session://data": SessionDataEvent;
  "session://state": SessionInfo;
  "session://agent_mark": AgentMarkEvent;
  "session://host_key": HostKeyEvent;
  "session://auth_prompt": AuthPromptEvent;
  /** A session's tunnels finished starting (each running or failed). */
  "session://tunnels": TunnelsEvent;
  "transfer://progress": TransferProgress;
  "edit://proposal": EditProposal;
  /** A proposal ended without the user (agent cancelled or timed out): drop its diff. */
  "edit://resolved": string;
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
  /** Deletes the vault from this device (works locked); closes every session. */
  vaultDelete(): Promise<void>;
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
  /** Saved secrets, without their values. */
  listSecrets(): Promise<SecretInfo[]>;
  /** Rename a secret, or replace its value (for everything that uses it). */
  updateSecret(id: string, label: string | null, value: string | null): Promise<void>;
  /** Rejects while a connection, identity, S3 account or provider still uses it. */
  deleteSecret(id: string): Promise<void>;
  listIdentities(): Promise<Identity[]>;
  /** Rejects without a name or user, or with `prompt` authentication. */
  saveIdentity(identity: Identity): Promise<Identity>;
  /** Rejects while a connection logs in with it. */
  deleteIdentity(id: string): Promise<void>;

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
  /** The connection's port forwards in this session; see "session://tunnels". */
  sessionTunnels(sessionId: string): Promise<TunnelStatus[]>;
  /** CPU, memory, disk, load and uptime of an SSH session's server (takes about half a second). */
  sessionStats(sessionId: string): Promise<ServerStats>;
  hostKeyDecision(requestId: string, accept: boolean): Promise<void>;

  // approvals
  approvalDecide(approvalId: string, decision: ApprovalDecision): Promise<void>;
  setApprovalMode(sessionId: string, mode: ApprovalMode): Promise<void>;
  getApprovalSettings(): Promise<ApprovalSettings>;
  saveApprovalSettings(settings: ApprovalSettings): Promise<void>;

  // providers / harnesses / agent
  // files (SFTP on an SSH session's connection)
  /** Start the session's root SFTP server through sudo (see `root` below). Rejects with
   *  `{ kind: "auth", message: "sudo_password_required" }` when the password must be typed. */
  sftpSudo(sessionId: string, password: string | null): Promise<void>;
  /** The file browser left root mode (agent proposals stop using root). */
  sftpRootOff(sessionId: string): Promise<void>;
  /** `root`: use the server started by `sftpSudo` instead of the login user's. */
  sftpHome(sessionId: string, root?: boolean): Promise<string>;
  sftpList(sessionId: string, path: string, root?: boolean): Promise<FileEntry[]>;
  sftpMkdir(sessionId: string, path: string, root?: boolean): Promise<void>;
  sftpRename(sessionId: string, from: string, to: string, root?: boolean): Promise<void>;
  sftpRemove(sessionId: string, path: string, isDir: boolean, root?: boolean): Promise<void>;
  /** Download into the Downloads folder; resolves with the local path. Progress: "transfer://progress". */
  sftpDownload(sessionId: string, path: string, transferId: string, root?: boolean): Promise<string>;
  /** Upload a local file into `dir`; resolves with the remote path. */
  sftpUpload(sessionId: string, localPath: string, dir: string, transferId: string, root?: boolean): Promise<string>;
  /**
   * Copy `path` from one file system into folder `toDir` of another: server to server (files
   * and folders), S3 to a server or a server to S3 (files). Streamed, nothing saved locally.
   * Resolves with the new path; rejects with `{ kind: "busy", message: "exists" }` when the
   * name is taken, unless `overwrite`.
   */
  copyBetween(from: Place, path: string, to: Place, toDir: string, overwrite: boolean, transferId: string): Promise<string>;

  // remote desktop
  /**
   * Open an RDP session at the view's size (device pixels; `scale` in percent). Desktop updates
   * arrive on `onMessage` as raw helper messages (util/rdp.ts parses them). Resolves to the id.
   */
  rdpOpen(connectionId: string, size: RdpSize, onMessage: (msg: ArrayBuffer) => void): Promise<string>;
  /** Keyboard and mouse input, batched. */
  rdpInput(id: string, events: RdpInput[]): Promise<void>;
  rdpResize(id: string, size: RdpSize): Promise<void>;
  rdpClose(id: string): Promise<void>;
  /** UTF-8 text up to 5 MB (binary files are refused). */
  sftpReadText(sessionId: string, path: string, root?: boolean): Promise<TextFile>;
  /**
   * Save in place. `expected`: the stamp from the read (null for a new file); a file changed
   * since rejects with `{ kind: "busy", message: "changed_on_server" }`. `backup` copies the
   * current file to `<path>.bak` first.
   */
  sftpWriteText(sessionId: string, path: string, text: string, expected: FileStamp | null, backup: boolean, root?: boolean): Promise<FileStamp>;
  /** Answer an agent's proposed edit (after saving it, when accepted). */
  editProposalDecide(proposalId: string, outcome: EditOutcome, note: string | null): Promise<void>;
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
  /** An object as text, for the read-only editor; rejects binaries and objects over 5 MB. */
  s3ReadText(accountId: string, path: string): Promise<TextFile>;
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
  /** Start OpenChamber for an OpenCode profile and open it in its own window. */
  openChamberOpen(sessionId: string, profileId: string): Promise<void>;
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
