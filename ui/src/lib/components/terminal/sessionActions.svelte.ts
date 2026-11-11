import type { Connection } from "../../api/types";
import type { MenuEntry } from "../common/ContextMenu.svelte";
import { t } from "../../i18n";
import { blankDbConnection, connections } from "../../stores/connections.svelte";
import { credentials } from "../../stores/credentials.svelte";
import { sessions } from "../../stores/sessions.svelte";
import { toasts } from "../../stores/toasts.svelte";
import { ui } from "../../stores/ui.svelte";
import { copyText } from "../../util/clipboard";
import { connectionAddress } from "../../util/sessionContext";

/** The connection whose read-only details dialog is open (one at a time, app-wide). */
class SessionDetails {
  connectionId = $state<string | null>(null);
  show(id: string) {
    this.connectionId = id;
  }
  close() {
    this.connectionId = null;
  }
}
export const sessionDetails = new SessionDetails();

/** The saved SSH connection behind a terminal session, if any. */
export function sshConnectionOf(sessionId: string | null | undefined): Connection | null {
  const cid = sessionId ? sessions.infos[sessionId]?.connection_id : null;
  const c = cid ? connections.byId.get(cid) : undefined;
  return c && c.kind === "ssh" ? c : null;
}

/** The user a connection logs in as (its identity's, when it uses one). */
export function loginUser(c: Connection): string {
  return (c.identity ? credentials.identity(c.identity)?.user : null) || c.user;
}

/** The jump hosts' names, in order (an id with no saved connection shows as is). */
export function jumpNames(c: Connection): string[] {
  return c.jump.map((id) => connections.byId.get(id)?.name ?? id);
}

async function copy(text: string) {
  if (await copyText(text)) toasts.success(t("sess.copied", { text }));
  else toasts.error(t("copy.failed"));
}

/** Quick actions for the terminal of an SSH session: files, another terminal, a split, copying
 *  its address, a database through it and its details. Empty for other sessions. */
export function quickActions(sessionId: string, { split = true }: { split?: boolean } = {}): MenuEntry[] {
  const c = sshConnectionOf(sessionId);
  if (!c) return [];
  const live = sessions.infos[sessionId]?.state !== "disconnected";
  const address = connectionAddress({ user: loginUser(c), host: c.host, port: c.port });
  return [
    { label: t("sess.openSftp"), icon: "folder", disabled: !live, action: () => sessions.openFiles(sessionId) },
    { label: t("sess.newTerminal"), icon: "terminal", action: () => void sessions.open(c.id).catch(toasts.error) },
    ...(split
      ? [
          {
            label: t("sess.splitRight"),
            icon: "splitRight" as const,
            action: () => {
              sessions.focusSession(sessionId);
              void sessions.open(c.id, "row").catch(toasts.error);
            },
          },
        ]
      : []),
    "sep",
    { label: t("sess.copyHost"), icon: "copy", action: () => void copy(c.host) },
    { label: t("sess.copyAddress", { address }), icon: "copy", action: () => void copy(address) },
    "sep",
    { label: t("sess.openDb"), icon: "database", action: () => ui.edit(blankDbConnection(c.group_id, c), true) },
    { label: t("sess.details"), icon: "info", action: () => sessionDetails.show(c.id) },
  ];
}
