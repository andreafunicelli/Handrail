import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import type { Api } from "./api";
import type { ApprovalSettings } from "./types";
import { errorMessage } from "./errors";

export const APPROVALS_KEY = "approvals";
export const DEFAULT_APPROVALS: ApprovalSettings = { default_mode: "confirm_all", deny_list: [] };

/** Thin invoke/listen wrapper. Command names and argument keys follow docs/ipc-contract.md exactly. */
export function createTauriApi(): Api {
  /** The update found by the last `checkForUpdate`, installed by `installUpdate`. */
  let pendingUpdate: Update | null = null;
  return {
    backend: "tauri",
    vaultStatus: () => invoke("vault_status"),
    vaultCreate: (password) => invoke("vault_create", { password }),
    vaultUnlock: (password, secret_key) => invoke("vault_unlock", { password, secret_key: secret_key || null }),
    vaultLock: () => invoke("vault_lock"),

    listConnections: () => invoke("list_connections"),
    saveConnection: (connection) => invoke("save_connection", { connection }),
    deleteConnection: (id) => invoke("delete_connection", { id }),
    listGroups: () => invoke("list_groups"),
    saveGroup: (group) => invoke("save_group", { group }),
    deleteGroup: (id) => invoke("delete_group", { id }),
    saveSecret: (secret) => invoke("save_secret", { secret }),

    openSession: (connection_id, cols, rows, prompt_ref) =>
      invoke("open_session", { connection_id, cols, rows, prompt_ref: prompt_ref ?? null }),
    answerAuthPrompt: (request_id, answers) => invoke("auth_prompt_answer", { request_id, answers }),
    openLocalSession: (shell, cols, rows) => invoke("open_local_session", { shell, cols, rows }),
    sessionWrite: (session_id, data_b64) => invoke("session_write", { session_id, data_b64 }),
    sessionResize: (session_id, cols, rows) => invoke("session_resize", { session_id, cols, rows }),
    closeSession: (session_id) => invoke("close_session", { session_id }),
    hostKeyDecision: (request_id, accept) => invoke("host_key_decision", { request_id, accept }),

    approvalDecide: (approval_id, decision) => invoke("approval_decide", { approval_id, decision }),
    setApprovalMode: (session_id, mode) => invoke("set_approval_mode", { session_id, mode }),
    // Approval policy is stored as one app setting; the core policy engine reads the same key.
    getApprovalSettings: async () =>
      (await invoke<ApprovalSettings | null>("get_setting", { key: APPROVALS_KEY })) ?? DEFAULT_APPROVALS,
    saveApprovalSettings: (settings) => invoke("set_setting", { key: APPROVALS_KEY, value: settings }),

    sftpHome: (session_id) => invoke("sftp_home", { session_id }),
    sftpList: (session_id, path) => invoke("sftp_list", { session_id, path }),
    sftpMkdir: (session_id, path) => invoke("sftp_mkdir", { session_id, path }),
    sftpRename: (session_id, from, to) => invoke("sftp_rename", { session_id, from, to }),
    sftpRemove: (session_id, path, is_dir) => invoke("sftp_remove", { session_id, path, is_dir }),
    sftpDownload: (session_id, path, transfer_id) => invoke("sftp_download", { session_id, path, transfer_id }),
    sftpUpload: (session_id, local_path, dir, transfer_id) => invoke("sftp_upload", { session_id, local_path, dir, transfer_id }),
    async pickFiles() {
      const picked = await openDialog({ multiple: true, directory: false });
      return picked == null ? [] : Array.isArray(picked) ? picked : [picked];
    },
    listSnippets: () => invoke("list_snippets"),
    saveSnippet: (snippet) => invoke("save_snippet", { snippet }),
    deleteSnippet: (id) => invoke("delete_snippet", { id }),
    listProviders: () => invoke("list_providers"),
    saveProvider: (provider, api_key) => invoke("save_provider", { provider, api_key: api_key || null }),
    deleteProvider: (id) => invoke("delete_provider", { id }),
    listModels: (provider_id) => invoke("list_models", { provider_id }),
    // No dedicated test command: a successful `list_models` proves base URL + key work.
    testProvider: async (provider_id) => {
      const t0 = performance.now();
      try {
        const models = await invoke<string[]>("list_models", { provider_id });
        return { ok: true, message: `${models.length} models · ${Math.round(performance.now() - t0)} ms` };
      } catch (e) {
        return { ok: false, message: errorMessage(e) };
      }
    },
    detectHarnesses: () => invoke("detect_harnesses"),
    listHarnessProfiles: () => invoke("list_harness_profiles"),
    saveHarnessProfile: (profile) => invoke("save_harness_profile", { profile }),
    deleteHarnessProfile: (id) => invoke("delete_harness_profile", { id }),
    listHarnessExtensions: (kind) => invoke("list_harness_extensions", { kind }),
    agentSend: (session_id, brain, message) => invoke("agent_send", { session_id, brain, message }),
    agentCancel: (run_id) => invoke("agent_cancel", { run_id }),
    harnessOpenNative: (session_id, profile_id, cols, rows) =>
      invoke("harness_open_native", { session_id, profile_id, cols, rows }),
    harnessLogin: (kind, cols, rows) => invoke("harness_login", { kind, cols, rows }),

    importDetect: () => invoke("import_detect"),
    importPreview: (args) => invoke("import_preview", { args }),
    importApply: (preview_id, skip_indices) => invoke("import_apply", { preview_id, skip_indices }),
    syncStatus: () => invoke("sync_status"),
    syncGithubStart: () => invoke("sync_configure_github_start"),
    syncGithubPoll: () => invoke("sync_configure_github_poll"),
    syncConfigureFolder: (path) => invoke("sync_configure_folder", { path }),
    syncJoin: () => invoke("sync_join"),
    syncNow: () => invoke("sync_now"),
    saveEmergencyKit: (contents) => invoke("save_emergency_kit", { contents }),

    appVersion: () => invoke("app_version"),
    checkForUpdate: async () => {
      pendingUpdate = await check();
      if (!pendingUpdate) return null;
      return {
        version: pendingUpdate.version,
        current_version: pendingUpdate.currentVersion,
        notes: pendingUpdate.body ?? null,
        date: pendingUpdate.date ?? null,
      };
    },
    installUpdate: async (progress) => {
      if (!pendingUpdate) throw { kind: "invalid", message: "no update to install; check again" };
      let total = 0;
      let done = 0;
      await pendingUpdate.downloadAndInstall((ev) => {
        if (ev.event === "Started") {
          total = ev.data.contentLength ?? 0;
          progress(total ? 0 : null);
        } else if (ev.event === "Progress") {
          done += ev.data.chunkLength;
          progress(total ? Math.min(1, done / total) : null);
        } else progress(1);
      });
    },
    restartApp: () => invoke("restart_app"),
    openUrl: (url) => invoke("open_url", { url }),
    syncSetAuto: (enabled) => invoke("sync_set_auto", { enabled }),
    syncDisconnect: () => invoke("sync_disconnect"),
    syncFolderPresets: () => invoke("sync_folder_presets"),
    vaultSnapshots: () => invoke("vault_snapshots"),
    vaultSnapshotCreate: () => invoke("vault_snapshot_create"),
    vaultSnapshotRestore: (name) => invoke("vault_snapshot_restore", { name }),
    vaultConflicts: () => invoke("vault_conflicts"),
    vaultConflictRestore: (file_id, recorded_at_ms) => invoke("vault_conflict_restore", { file_id, recorded_at_ms }),
    vaultConflictsClear: () => invoke("vault_conflicts_clear"),

    on: async (event, cb) => {
      const un = await listen(event, (e) => cb(e.payload as never));
      return un;
    },
  };
}
