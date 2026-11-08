import { api } from "../api";
import type { ApprovalDecision, DbApprovalRequest } from "../api/types";
import { t } from "../i18n";
import { toasts } from "./toasts.svelte";

/** Agent SQL waiting for the user, shown in the database tab it targets. Listened to for the
 *  whole app, so a request for a tab in the background is not missed. */
class DbApprovals {
  items = $state<DbApprovalRequest[]>([]);
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

  async decide(id: string, decision: ApprovalDecision) {
    try {
      await api.dbApprovalDecide(id, decision);
    } catch (e) {
      toasts.error(e);
    } finally {
      this.drop(id);
    }
  }

  private drop(id: string) {
    this.items = this.items.filter((i) => i.approval_id !== id);
  }
}

export const dbApprovals = new DbApprovals();
