import { isIpcError } from "../api/errors";
import type { SessionInfo, SshAuth } from "../api/types";

/** Attempts before giving up (about eight minutes with the delays below). */
export const RECONNECT_ATTEMPTS = 20;

/** Milliseconds to wait before attempt `n` (0-based): now, then 1, 2, 4, 8, 16 and 30 s. */
export function reconnectDelay(n: number): number {
  return n === 0 ? 0 : Math.min(30, 2 ** (n - 1)) * 1000;
}

/** Failures worth another try (the backend's rule): the network, not the login or host key. */
export function retryable(e: unknown): boolean {
  return isIpcError(e) && ["network", "io", "other"].includes(e.kind);
}

/**
 * Whether a session should reconnect by itself: an SSH session whose connection was lost (it
 * did not end with an exit status, e.g. from `exit`), logging in without asking the user.
 */
export function shouldAutoReconnect(info: SessionInfo, auth: SshAuth["type"] | undefined): boolean {
  return (
    info.state === "disconnected" &&
    !!info.connection_id &&
    !!info.ended &&
    info.ended.exit_status == null &&
    !!auth &&
    auth !== "prompt"
  );
}
