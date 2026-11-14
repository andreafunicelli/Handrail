import { api } from "../api";
import { errorMessage, isIpcError } from "../api/errors";
import type { ApprovalDecision, DbApprovalRequest } from "../api/types";
import { t } from "../i18n";
import { toasts } from "./toasts.svelte";

/** Agent SQL waiting for the user, shown in the database tab it targets. Listened to for the
 *  whole app, so a request for a tab in the background is not missed. */
class DbApprovals {
  items = $state<DbApprovalRequest[]>([]);
  /** Why the last answer to a request did not reach the backend, by approval id. */
  failed = $state<Record<string, string>>({});
  /** The pane of each open database tab (its layout leaf) and its name, for the AI sidebar's
   *  "waiting" banner. Set by DbView. */
  tabs = $state<Record<string, { leaf: string; name: string }>>({});
  private started = false;

  async init() {
    if (this.started) return;
    this.started = true;
    await api.on("db://approval", (req) => {
      if (this.items.some((i) => i.approval_id === req.approval_id)) return;
      this.items.push(req);
      toasts.info(t("db.agentAsks", { who: req.actor_label }));
    });
    await api.on("db://approval-resolved", (id) => this.drop(id));
  }

  forTab(databaseId: string | null): DbApprovalRequest[] {
    return databaseId ? this.items.filter((i) => i.database_id === databaseId) : [];
  }

  /** Answer a request. Returns whether the backend took it: on failure the request stays on
   *  screen (with the error) so the user can answer again. */
  async decide(id: string, decision: ApprovalDecision): Promise<boolean> {
    delete this.failed[id];
    try {
      await api.dbApprovalDecide(id, decision);
      this.drop(id);
      return true;
    } catch (e) {
      // The backend no longer has the request (timed out, tab closed): nothing left to answer.
      if (isIpcError(e) && e.kind === "not_found") {
        this.drop(id);
        toasts.error(e);
      } else this.failed[id] = errorMessage(e);
      return false;
    }
  }

  /** A database tab closed: its requests can no longer be answered from its pane. */
  forget(databaseId: string) {
    delete this.tabs[databaseId];
    for (const i of this.items) if (i.database_id === databaseId) delete this.failed[i.approval_id];
    this.items = this.items.filter((i) => i.database_id !== databaseId);
  }

  private drop(id: string) {
    this.items = this.items.filter((i) => i.approval_id !== id);
    delete this.failed[id];
  }
}

export const dbApprovals = new DbApprovals();
