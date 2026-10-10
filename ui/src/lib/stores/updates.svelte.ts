import { api } from "../api";
import type { UpdateInfo } from "../api/types";
import { errorMessage } from "../api/errors";

/** First check shortly after start, then every 6 hours while the app runs. */
const FIRST_CHECK_MS = 15_000;
const INTERVAL_MS = 6 * 3_600_000;

export type UpdatePhase = "idle" | "checking" | "available" | "downloading" | "ready" | "error";

/**
 * Checks for new releases automatically but never installs on its own: installing restarts the
 * app, which would cut every open SSH session. The user picks the moment.
 */
class Updates {
  phase = $state<UpdatePhase>("idle");
  info = $state<UpdateInfo | null>(null);
  /** 0..1 while downloading, null when the size is unknown. */
  progress = $state<number | null>(null);
  error = $state<string | null>(null);
  lastChecked = $state<number | null>(null);
  /** The user closed the banner for this version. */
  dismissed = $state<string | null>(null);
  private started = false;

  /** Start background checks. Skipped in the browser demo and in `vite dev` builds. */
  start(auto = api.backend === "tauri" && !import.meta.env.DEV) {
    if (this.started || !auto) return;
    this.started = true;
    setTimeout(() => void this.check(), FIRST_CHECK_MS);
    setInterval(() => void this.check(), INTERVAL_MS);
  }

  /** Returns whether an update is available. Background checks fail quietly. */
  async check(manual = false): Promise<boolean> {
    if (this.phase === "checking" || this.phase === "downloading" || this.phase === "ready") return !!this.info;
    this.phase = "checking";
    this.error = null;
    try {
      this.info = await api.checkForUpdate();
      this.lastChecked = Date.now();
      this.phase = this.info ? "available" : "idle";
      return !!this.info;
    } catch (e) {
      this.phase = manual ? "error" : this.info ? "available" : "idle";
      if (manual) this.error = errorMessage(e);
      return false;
    }
  }

  async install() {
    if (!this.info) return;
    this.phase = "downloading";
    this.progress = 0;
    this.error = null;
    try {
      await api.installUpdate((p) => (this.progress = p));
      this.phase = "ready";
      await api.restartApp();
    } catch (e) {
      this.phase = "error";
      this.error = errorMessage(e);
    }
  }

  dismiss() {
    this.dismissed = this.info?.version ?? null;
  }

  get bannerVisible() {
    return (
      (this.phase === "available" && !!this.info && this.dismissed !== this.info.version) ||
      this.phase === "downloading" ||
      this.phase === "ready" ||
      (this.phase === "error" && !!this.info)
    );
  }
}

export const updates = new Updates();
