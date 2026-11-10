import type { Environment, SessionInfo } from "../../api/types";
import { fmtDateTime } from "../../i18n";
import { connections } from "../../stores/connections.svelte";
import { sessions } from "../../stores/sessions.svelte";
import { terminals } from "../../terminal/host";

/** Where an agent works: the session's connection, environment and effective user. */
export interface Target {
  sessionId: string;
  name: string;
  env: Environment;
  /** The user the shell runs as: what the prompt reports, else the connection's login user. */
  user: string | null;
  host: string | null;
  connectionId: string | null;
  local: boolean;
  info: SessionInfo | null;
}

/** Reads reactive stores: call it inside `$derived`. */
export function targetOf(sessionId: string): Target | null {
  const info = sessions.infos[sessionId] ?? null;
  const conn = info?.connection_id ? connections.byId.get(info.connection_id) : undefined;
  if (!info && !conn) return null;
  return {
    sessionId,
    name: conn?.name ?? info?.title ?? sessionId,
    env: info?.env ?? conn?.env ?? "none",
    user: info?.user ?? conn?.user ?? null,
    host: info?.host ?? conn?.host ?? null,
    connectionId: info?.connection_id ?? null,
    local: !info?.connection_id,
    info,
  };
}

/** Focus the session's terminal and, when given, scroll to where `command` ran. */
export function jumpToTerminal(sessionId: string, command?: string | null) {
  sessions.focusSession(sessionId);
  if (command) requestAnimationFrame(() => terminals.peek(sessionId)?.jumpTo(command));
}

export function fmtMs(ms: number) {
  return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(ms < 10000 ? 1 : 0)} s`;
}

/** Time of day with seconds, in the interface language. */
export function fmtClock(at: number) {
  return fmtDateTime(at, { timeStyle: "medium" });
}
