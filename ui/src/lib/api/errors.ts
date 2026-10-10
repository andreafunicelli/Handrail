import type { IpcError } from "./types";

/** Backend commands reject with `{ kind, message }`; anything else is normalized here. */
export function isIpcError(e: unknown): e is IpcError {
  return !!e && typeof e === "object" && "kind" in e && "message" in e;
}

export function errorMessage(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  if (e && typeof e === "object" && "message" in e) return String((e as { message: unknown }).message);
  return "Unexpected error";
}

/** `vault_unlock` rejects with this when the device has no Secret Key yet. */
export const isSecretKeyRequired = (e: unknown) => isIpcError(e) && e.kind === "auth" && e.message === "secret_key_required";
