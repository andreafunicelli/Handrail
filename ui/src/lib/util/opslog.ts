/**
 * Operations history: what agents did, kept as metadata only (never terminal output, keys or
 * vault secrets). Pure helpers: redaction, record building, retention and filtering.
 */
import type { ApprovalDecision, CommandResult, RiskClass } from "../api/types";

export const MASK = "***";
export const MAX_OPERATION_LEN = 300;

export type OpsDecision = "approved" | "edited" | "denied" | "auto" | "blocked" | "none";
export type OpsErrorCategory = "exit_code" | "timed_out" | "interrupted" | "needs_user" | "denied" | "rejected" | "tool_error" | "no_result";

export interface OpsEntry {
  /** Epoch ms when the call started. */
  at: number;
  session_id: string;
  connection_id: string | null;
  connection_name: string | null;
  env: string | null;
  agent: string | null;
  tool: string;
  /** The command, SQL or path the tool worked on, redacted and truncated; null when command
   *  text is not recorded. */
  operation: string | null;
  decision: OpsDecision;
  risk: RiskClass | null;
  /** CommandState, a file-edit outcome (saved/rejected/failed), "ok", "error" or "no_result". */
  outcome: string;
  exit_code: number | null;
  duration_ms: number | null;
  error: OpsErrorCategory | null;
}

export interface OpsLogSettings {
  enabled: boolean;
  record_commands: boolean;
  retention_days: number;
  max_entries: number;
}

export const DEFAULT_OPSLOG_SETTINGS: OpsLogSettings = {
  enabled: true,
  record_commands: true,
  retention_days: 30,
  max_entries: 1000,
};

const DAY = 86_400_000;
const VALUE = `("[^"]*"|'[^']*'|[^\\s"'&;|]+)`;

/** Known secret shapes, applied in order (key blocks first: they span lines). */
const RULES: [RegExp, string | ((...m: string[]) => string)][] = [
  // PEM private key blocks (also an unterminated one, cut by truncation).
  [/-----BEGIN ([A-Z0-9 ]*)PRIVATE KEY-----[\s\S]*?(?:-----END [A-Z0-9 ]*PRIVATE KEY-----|$)/g, (_m, kind) => `-----BEGIN ${kind}PRIVATE KEY----- ${MASK} -----END ${kind}PRIVATE KEY-----`],
  // user:password@ in URLs (mysql://, postgres://, https://, redis://…).
  [/\b([a-z][a-z0-9+.-]*:\/\/[^\s:/@]*):[^\s@/]+@/gi, `$1:${MASK}@`],
  // HTTP authorization headers.
  [/\b(authorization\s*:\s*(?:bearer|basic|token|digest)?\s*)[^\s"']+/gi, `$1${MASK}`],
  [/\b(x-api-key|api-key|x-auth-token|private-token)(\s*:\s*)[^\s"']+/gi, `$1$2${MASK}`],
  // curl -u user:password / --user user:password.
  [/(\s(?:-u|--user)[\s=]+)([^\s:]+):(\S+)/g, `$1$2:${MASK}`],
  // sshpass -p secret.
  [/\b(sshpass\s+-p\s*)\S+/g, `$1${MASK}`],
  // mysql -p<secret> (attached value; a bare -p prompts and is kept).
  [/(^|\s)-p(?=[^\s-])\S+/g, `$1-p${MASK}`],
  // --password=x, --password x, --token x, --secret x, --api-key x…
  [new RegExp(`(--(?:password|passwd|pass|pwd|token|secret|api[-_]?key|access[-_]?key|secret[-_]?key|auth[-_]?token|client[-_]?secret)(?:=|\\s+))${VALUE}`, "gi"), `$1${MASK}`],
  // NAME=value where NAME looks secret (PASSWORD=, PGPASSWORD=, AWS_SECRET_ACCESS_KEY=, token=…).
  [new RegExp(`\\b([A-Za-z0-9_.-]*(?:password|passwd|pwd|token|secret|api_?key|apikey|access_?key|private_?key|credential)[A-Za-z0-9_]*)(\\s*[=:]\\s*)${VALUE}`, "gi"), `$1$2${MASK}`],
  // SQL: IDENTIFIED BY 'x', PASSWORD 'x'.
  [/\b(identified\s+(?:with\s+\S+\s+)?by\s+)('[^']*'|"[^"]*"|\S+)/gi, `$1'${MASK}'`],
  [/\b(password\s*\(?\s*)('[^']*'|"[^"]*")/gi, `$1'${MASK}'`],
  // SET PASSWORD [FOR user] = 'x'.
  [/\b(set\s+password\b[^=;]*=\s*)('[^']*'|"[^"]*")/gi, `$1'${MASK}'`],
  // Cloud and service keys.
  [/\b(AKIA|ASIA)[0-9A-Z]{16}\b/g, `$1${MASK}`],
  [/\b(gh[pousr]_)[A-Za-z0-9]{20,}\b/g, `$1${MASK}`],
  [/\b(github_pat_)[A-Za-z0-9_]{20,}\b/g, `$1${MASK}`],
  [/\b(xox[abprs]-)[A-Za-z0-9-]{10,}/g, `$1${MASK}`],
  [/\b(sk-(?:ant-|proj-)?)[A-Za-z0-9_-]{16,}/g, `$1${MASK}`],
  [/\b(glpat-)[A-Za-z0-9_-]{16,}/g, `$1${MASK}`],
];

/** Mask known secret patterns in a command (best effort: unknown formats are not detected). */
export function redact(text: string): string {
  let out = text;
  for (const [re, by] of RULES) out = out.replace(re, by as string);
  return out;
}

/** Redact, then cut to `max` characters (the cut never exposes a half-masked secret: redaction
 *  runs on the whole text first). */
export function redactOperation(text: string, max = MAX_OPERATION_LEN): string {
  const r = redact(text.replace(/\r\n?/g, "\n"));
  return r.length > max ? r.slice(0, max - 1) + "…" : r;
}

/** The command, SQL or path a tool call worked on, or the tool's other arguments. */
export function operationText(tool: string, input: Record<string, unknown>, result: unknown): string {
  const str = (v: unknown) => (typeof v === "string" ? v : null);
  if (tool === "run_command") {
    const r = result && typeof result === "object" ? (result as Partial<CommandResult>) : null;
    return str(r?.command) ?? str(input.command) ?? "";
  }
  if (tool === "send_keys") return str(input.keys) ?? str(input.text) ?? str(input.data) ?? "";
  if (tool === "propose_file_edit") return str(input.path) ?? "";
  if (tool === "db_query") return str(input.sql) ?? str(input.query) ?? "";
  // Other tools: their arguments, without the session id and any file contents.
  const rest = Object.entries(input).filter(([k, v]) => k !== "session_id" && k !== "content" && v != null && v !== "");
  return rest.map(([k, v]) => `${k}=${typeof v === "string" ? v : JSON.stringify(v)}`).join(" ");
}

export function decisionOf(tool: string, approval: { decision: ApprovalDecision | null } | null, result: unknown): OpsDecision {
  const d = approval?.decision;
  if (d) return d.type === "approve" ? "approved" : d.type === "approve_edited" ? "edited" : "denied";
  if (tool !== "run_command") return "none";
  const state = result && typeof result === "object" ? (result as Partial<CommandResult>).state : undefined;
  // Denied without an approval card: the policy refused it (or the request expired).
  return state === "denied" ? "blocked" : "auto";
}

/** Outcome and error category of a finished call. */
export function outcomeOf(result: unknown, isError: boolean, aborted: boolean): { outcome: string; exit_code: number | null; duration_ms: number | null; error: OpsErrorCategory | null } {
  if (aborted && result == null) return { outcome: "no_result", exit_code: null, duration_ms: null, error: "no_result" };
  const r = result && typeof result === "object" ? (result as Record<string, unknown>) : null;
  if (r && typeof r.state === "string") {
    const c = r as unknown as CommandResult;
    const exit = typeof c.exit_code === "number" ? c.exit_code : null;
    const error: OpsErrorCategory | null =
      c.state === "completed" ? (exit != null && exit !== 0 ? "exit_code" : null) : (c.state as OpsErrorCategory);
    return { outcome: c.state, exit_code: exit, duration_ms: typeof c.duration_ms === "number" ? c.duration_ms : null, error: isError && !error ? "tool_error" : error };
  }
  if (r && typeof r.outcome === "string") {
    const o = r.outcome;
    return { outcome: o, exit_code: null, duration_ms: null, error: o === "rejected" ? "rejected" : o === "failed" ? "tool_error" : isError ? "tool_error" : null };
  }
  return { outcome: isError ? "error" : "ok", exit_code: null, duration_ms: null, error: isError ? "tool_error" : null };
}

/** Drop entries older than the retention window, then keep the newest `max_entries`. */
export function prune(entries: readonly OpsEntry[], settings: Pick<OpsLogSettings, "retention_days" | "max_entries">, now = Date.now()): OpsEntry[] {
  const since = now - Math.max(1, settings.retention_days) * DAY;
  const kept = entries.filter((e) => e.at >= since).sort((a, b) => a.at - b.at);
  const max = Math.max(1, settings.max_entries);
  return kept.length > max ? kept.slice(kept.length - max) : kept;
}

/** Validate what the vault returned (an older or foreign value must not break the page). */
export function parseEntries(v: unknown): OpsEntry[] {
  if (!Array.isArray(v)) return [];
  return v.filter((e): e is OpsEntry => !!e && typeof e === "object" && typeof (e as OpsEntry).at === "number" && typeof (e as OpsEntry).tool === "string");
}

export function parseSettings(v: unknown): OpsLogSettings {
  const s = v && typeof v === "object" ? (v as Partial<OpsLogSettings>) : {};
  const num = (x: unknown, d: number, lo: number, hi: number) => (typeof x === "number" && Number.isFinite(x) ? Math.min(hi, Math.max(lo, Math.round(x))) : d);
  return {
    enabled: typeof s.enabled === "boolean" ? s.enabled : DEFAULT_OPSLOG_SETTINGS.enabled,
    record_commands: typeof s.record_commands === "boolean" ? s.record_commands : DEFAULT_OPSLOG_SETTINGS.record_commands,
    retention_days: num(s.retention_days, DEFAULT_OPSLOG_SETTINGS.retention_days, 1, 3650),
    max_entries: num(s.max_entries, DEFAULT_OPSLOG_SETTINGS.max_entries, 10, 10000),
  };
}

export interface OpsFilter {
  connection: string; // "" = all, else connection id (or "local" for sessions without one)
  decision: OpsDecision | "";
  outcome: "" | "ok" | "failed";
  text: string;
}

export function isFailure(e: OpsEntry): boolean {
  return e.error != null && e.error !== "denied" && e.error !== "rejected";
}

export function filterEntries(entries: readonly OpsEntry[], f: OpsFilter): OpsEntry[] {
  const q = f.text.trim().toLowerCase();
  return entries.filter(
    (e) =>
      (!f.connection || (e.connection_id ?? "local") === f.connection) &&
      (!f.decision || e.decision === f.decision) &&
      (!f.outcome || (f.outcome === "failed" ? isFailure(e) : !isFailure(e))) &&
      (!q || [e.operation, e.tool, e.connection_name, e.agent, e.outcome].some((s) => s?.toLowerCase().includes(q))),
  );
}
