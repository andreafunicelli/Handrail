import { api } from "../api";
import type { VaultStatus } from "../api/types";
import { t } from "../i18n";
import { clearUserContent } from "../util/storage";
import { opsLog } from "./opsLog.svelte";
import { toasts } from "./toasts.svelte";
import { transfers } from "./transfers.svelte";
import { ui } from "./ui.svelte";

/** Work that locking would lose, by owner (a database pane's unsaved edits, an import running):
 *  locking closes database and remote desktop panes. Set with `setUnsaved`. */
const unsaved = new Set<string>();
export function setUnsaved(key: string, on: boolean) {
  if (on) unsaved.add(key);
  else unsaved.delete(key);
}
/** Whether locking now would lose work (unsaved edits, an import, export or file transfer). */
export const hasUnsavedWork = () => unsaved.size > 0 || transfers.running > 0;

export type VaultPhase = "loading" | "create" | "join" | "kit" | "unlock" | "ready";

class Vault {
  phase = $state<VaultPhase>("loading");
  status = $state<VaultStatus>({ initialized: false, locked: true });
  /** Shown once on the Emergency Kit screen, then dropped from memory. */
  secretKey = $state<string | null>(null);
  /** This device just joined a synced vault: unlocking needs the Secret Key. */
  joined = $state(false);
  /** A vault was just created here: the app opens with the guided tour. */
  justCreated = $state(false);

  async refresh() {
    this.status = await api.vaultStatus();
    this.phase = !this.status.initialized ? "create" : this.status.locked ? "unlock" : "ready";
  }
  /** After `sync_join` downloaded the manifest of a vault created on another device. */
  async afterJoin() {
    this.joined = true;
    await this.refresh();
  }
  async create(password: string) {
    const { secret_key } = await api.vaultCreate(password);
    this.secretKey = secret_key;
    this.justCreated = true;
    this.phase = "kit";
  }
  finishKit() {
    this.secretKey = null;
    this.phase = "ready";
  }
  async unlock(password: string, secretKey?: string) {
    await api.vaultUnlock(password, secretKey);
    this.joined = false;
    this.phase = "ready";
  }
  async lock() {
    // Operation records still batched in memory go into the vault while it is open.
    await opsLog.flush().catch(() => {});
    await api.vaultLock();
    this.phase = "unlock";
  }
  /** Lock on the user's request: asks first when unsaved work would be lost. Never throws. */
  async lockAsking() {
    if (hasUnsavedWork() && !(await ui.ask(t("vault.lockUnsaved.title"), { body: t("vault.lockUnsaved.body"), confirmLabel: t("vault.lock"), danger: true }))) return;
    await this.lock().catch(toasts.error);
  }
  /** Delete the vault, then start the app over: every store still caches its items and tabs. */
  async erase() {
    await api.vaultDelete();
    // Query tabs and history, workspaces, recent and pinned connections live outside the vault.
    clearUserContent();
    location.reload();
  }
}

export const vault = new Vault();

/** 0..4 password strength estimate (length + character classes + obvious patterns). */
export function passwordStrength(pw: string): 0 | 1 | 2 | 3 | 4 {
  if (pw.length < 10) return 0;
  let classes = 0;
  if (/[a-z]/.test(pw)) classes++;
  if (/[A-Z]/.test(pw)) classes++;
  if (/\d/.test(pw)) classes++;
  if (/[^A-Za-z0-9]/.test(pw)) classes++;
  let score = (pw.length >= 14 ? 2 : 1) + (classes >= 3 ? 1 : 0) + (pw.length >= 20 || (classes === 4 && pw.length >= 16) ? 1 : 0);
  if (/^(.)\1+$/.test(pw) || /password|1234|qwerty/i.test(pw)) score = Math.min(score, 1);
  return Math.max(1, Math.min(4, score)) as 1 | 2 | 3 | 4;
}
