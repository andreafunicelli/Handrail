import type { Api } from "./api";
import { createTauriApi } from "./tauri";

export const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** No backend: a production build opened outside the app. Every call fails (main.ts says why). */
const noBackend = new Proxy({} as Api, {
  get: () => () => Promise.reject(new Error("no backend")),
});

/** The single backend instance: real Tauri IPC inside the app, an in-browser mock in development
 *  and tests only (loaded on demand, so production builds do not contain it). */
export const api: Api = isTauri ? createTauriApi() : import.meta.env.DEV ? (await import("./mock")).createMockApi() : noBackend;
