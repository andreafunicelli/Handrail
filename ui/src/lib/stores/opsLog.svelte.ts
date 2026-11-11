import { api } from "../api";
import { errorMessage } from "../api/errors";
import type { ApprovalDecision, RiskClass } from "../api/types";
import { uuid } from "../util/id";
import {
  DEFAULT_OPSLOG_SETTINGS,
  decisionOf,
  operationText,
  outcomeOf,
  parseEntries,
  parseSettings,
  prune,
  redactOperation,
  type OpsEntry,
  type OpsLogSettings,
} from "../util/opslog";
import { load, save } from "../util/storage";
import { connections } from "./connections.svelte";
import { sessions } from "./sessions.svelte";

export const OPSLOG_SETTINGS_KEY = "opslog.settings";
/** Writes are batched: at most one vault write per this interval. */
const FLUSH_MS = 5000;
/** A locked vault cannot take writes: retry later, keeping at most this many records in memory. */
const RETRY_MS = 30_000;

/** A random id for this device, so each device keeps its own record list (no sync conflicts). */
function deviceId(): string {
  let id = load<string | null>("device-id", null);
  if (!id) {
    id = uuid();
    save("device-id", id);
  }
  return id;
}

export interface OpsRecordInput {
  sessionId: string;
  tool: string;
  input: Record<string, unknown>;
  output: unknown;
  isError: boolean;
  aborted: boolean;
  actor: string | null;
  approval: { decision: ApprovalDecision | null; risk: RiskClass } | null;
  startedAt: number;
  endedAt: number;
}

/**
 * The operations history of this device: encrypted in the vault under `opslog.<device id>`
 * (generic setting store), capped and pruned by retention. Records hold metadata and the
 * redacted command only, never terminal output.
 */
class OpsLog {
  settings = $state<OpsLogSettings>({ ...DEFAULT_OPSLOG_SETTINGS });
  /** What the vault holds (after the last load or write). */
  stored = $state<OpsEntry[]>([]);
  /** Recorded, not yet written. */
  pending = $state<OpsEntry[]>([]);
  loaded = $state(false);
  error = $state<string | null>(null);

  /** Stored and pending, oldest first. */
  all = $derived([...this.stored, ...this.pending]);

  private timer: ReturnType<typeof setTimeout> | null = null;
  private writing: Promise<void> | null = null;
  private settingsLoaded = false;

  get key() {
    return `opslog.${deviceId()}`;
  }

  /** Read the settings early so the first records already follow them. Never throws. */
  async init() {
    if (this.settingsLoaded) return;
    try {
      this.settings = parseSettings(await api.getSetting(OPSLOG_SETTINGS_KEY));
      this.settingsLoaded = true;
    } catch {
      /* locked or unavailable: defaults until load() */
    }
  }

  /** Load settings and records from the vault (fails, and shows nothing, while it is locked). */
  async load() {
    try {
      const [s, list] = await Promise.all([api.getSetting(OPSLOG_SETTINGS_KEY), api.getSetting(this.key)]);
      this.settings = parseSettings(s);
      this.settingsLoaded = true;
      this.stored = prune(parseEntries(list), this.settings);
      this.loaded = true;
      this.error = null;
    } catch (e) {
      this.stored = [];
      this.loaded = false;
      this.error = errorMessage(e);
    }
  }

  /** Called when a tool call finishes. Cheap: builds one small record and schedules a write. */
  record(r: OpsRecordInput) {
    if (!this.settings.enabled) return;
    const info = sessions.infos[r.sessionId];
    const conn = info?.connection_id ? connections.byId.get(info.connection_id) : undefined;
    const out = outcomeOf(r.output, r.isError, r.aborted);
    const op = operationText(r.tool, r.input, r.output);
    const entry: OpsEntry = {
      at: r.startedAt,
      session_id: r.sessionId,
      connection_id: info?.connection_id ?? null,
      connection_name: conn?.name ?? info?.title ?? null,
      env: info?.env ?? conn?.env ?? null,
      agent: r.actor,
      tool: r.tool,
      operation: this.settings.record_commands && op ? redactOperation(op) : null,
      decision: decisionOf(r.tool, r.approval, r.output),
      risk: r.approval?.risk ?? null,
      outcome: out.outcome,
      exit_code: out.exit_code,
      duration_ms: out.duration_ms ?? Math.max(0, r.endedAt - r.startedAt),
      error: out.error,
    };
    this.pending.push(entry);
    if (this.pending.length > this.settings.max_entries) this.pending.splice(0, this.pending.length - this.settings.max_entries);
    this.schedule(FLUSH_MS);
  }

  private schedule(ms: number) {
    if (this.timer) return;
    this.timer = setTimeout(() => {
      this.timer = null;
      void this.flush();
    }, ms);
  }

  /** Write pending records (merged with what the vault holds). */
  async flush(): Promise<void> {
    if (this.writing) return this.writing;
    if (!this.pending.length) return;
    this.writing = (async () => {
      const batch = $state.snapshot(this.pending) as OpsEntry[];
      try {
        // Read before writing: the page may never have loaded, and the settings may have changed.
        const [s, list] = await Promise.all([api.getSetting(OPSLOG_SETTINGS_KEY), api.getSetting(this.key)]);
        this.settings = parseSettings(s);
        this.settingsLoaded = true;
        let add = batch;
        if (!this.settings.enabled) add = [];
        else if (!this.settings.record_commands) add = batch.map((e) => ({ ...e, operation: null }));
        const merged = prune([...parseEntries(list), ...add], this.settings);
        if (add.length) await api.setSetting(this.key, merged);
        this.pending.splice(0, batch.length);
        this.stored = merged;
        this.loaded = true;
        this.error = null;
      } catch {
        // Locked vault (or no backend): keep the records and try again later.
        this.schedule(RETRY_MS);
      }
    })();
    try {
      await this.writing;
    } finally {
      this.writing = null;
    }
  }

  async saveSettings(next: OpsLogSettings) {
    const s = parseSettings(next);
    // As for clear: a write under way would otherwise overwrite the pruned history.
    await this.writing;
    await api.setSetting(OPSLOG_SETTINGS_KEY, s);
    this.settings = s;
    if (!s.enabled) this.pending = [];
    else if (!s.record_commands) this.pending = this.pending.map((e) => ({ ...e, operation: null }));
    // A shorter retention or a smaller cap applies to what is stored at once (turning command
    // text off applies to new records; Clear history removes the old ones).
    const pruned = prune(this.stored, s);
    if (this.loaded && pruned.length !== this.stored.length) {
      await api.setSetting(this.key, pruned);
      this.stored = pruned;
    }
  }

  /** Delete this device's history (and what has not been written yet). A write already under
   *  way finishes first: it read the old history, and would put it back after the clear. */
  async clear() {
    if (this.timer) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    await this.writing;
    await api.setSetting(this.key, []);
    this.pending = [];
    this.stored = [];
  }
}

export const opsLog = new OpsLog();
