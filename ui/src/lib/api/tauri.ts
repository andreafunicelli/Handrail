import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Api } from "./api";

/** Thin invoke/listen wrapper. Command names and argument keys follow docs/ipc-contract.md exactly. */
export function createTauriApi(): Api {
  return {
    backend: "tauri",
    vaultStatus: () => invoke("vault_status"),
    vaultCreate: (password) => invoke("vault_create", { password }),
    vaultUnlock: (password, secret_key) => invoke("vault_unlock", { password, secret_key: secret_key || null }),
    vaultLock: () => invoke("vault_lock"),
    vaultSetAutoLock: (minutes) => invoke("vault_set_auto_lock", { minutes }),

    listConnections: () => invoke("list_connections"),
    saveConnection: (connection) => invoke("save_connection", { connection }),
    deleteConnection: (id) => invoke("delete_connection", { id }),
    listGroups: () => invoke("list_groups"),
    saveGroup: (group) => invoke("save_group", { group }),
    deleteGroup: (id) => invoke("delete_group", { id }),
    saveSecret: (secret) => invoke("save_secret", { secret }),

    openSession: (connection_id, cols, rows) => invoke("open_session", { connection_id, cols, rows }),
    openLocalSession: (shell, cols, rows) => invoke("open_local_session", { shell, cols, rows }),
    sessionWrite: (session_id, data_b64) => invoke("session_write", { session_id, data_b64 }),
    sessionResize: (session_id, cols, rows) => invoke("session_resize", { session_id, cols, rows }),
    closeSession: (session_id) => invoke("close_session", { session_id }),
    listSessions: () => invoke("list_sessions"),
    hostKeyDecision: (request_id, accept) => invoke("host_key_decision", { request_id, accept }),

    approvalDecide: (approval_id, decision) => invoke("approval_decide", { approval_id, decision }),
    setApprovalMode: (session_id, mode) => invoke("set_approval_mode", { session_id, mode }),
    getApprovalSettings: () => invoke("get_approval_settings"),
    saveApprovalSettings: (settings) => invoke("save_approval_settings", { settings }),

    listProviders: () => invoke("list_providers"),
    saveProvider: (provider, api_key) => invoke("save_provider", { provider, api_key: api_key || null }),
    deleteProvider: (id) => invoke("delete_provider", { id }),
    listModels: (provider_id) => invoke("list_models", { provider_id }),
    testProvider: (provider_id) => invoke("test_provider", { provider_id }),
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
    importPreview: (arg) => invoke("import_preview", arg),
    importApply: (preview_id, skip_indices) => invoke("import_apply", { preview_id, skip_indices }),
    syncStatus: () => invoke("sync_status"),
    syncGithubStart: () => invoke("sync_configure_github_start"),
    syncGithubPoll: () => invoke("sync_configure_github_poll"),
    syncConfigureFolder: (path) => invoke("sync_configure_folder", { path }),
    syncNow: () => invoke("sync_now"),

    on: async (event, cb) => {
      const un = await listen(event, (e) => cb(e.payload as never));
      return un;
    },
  };
}
