import type { Api } from "./api";
import { createMockApi } from "./mock";
import { createTauriApi } from "./tauri";

export const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** The single backend instance: real Tauri IPC inside the app, an in-browser mock otherwise. */
export const api: Api = isTauri ? createTauriApi() : createMockApi();
