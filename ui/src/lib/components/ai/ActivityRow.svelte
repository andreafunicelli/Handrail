<script lang="ts">
  import Icon from "../icons/Icon.svelte";
  import { t, type MessageKey } from "../../i18n";
  import { isFailedExec, type ToolActivity } from "../../util/agentActivity";
  import { fmtClock, fmtMs, jumpToTerminal, type Target } from "./target";

  let { entry, sessionId, target, compact = false }: { entry: ToolActivity; sessionId: string; target: Target | null; compact?: boolean } = $props();
  let open = $state(false);

  const failed = $derived(isFailedExec(entry.exec));
  const tone = $derived(
    entry.exec === "running" || entry.exec === "waiting"
      ? "accent"
      : failed
        ? "warn"
        : entry.exec === "denied" || entry.exec === "rejected" || entry.exec === "needs_user"
          ? ""
          : "ok",
  );
  const apprTone = $derived(
    entry.approvalStatus === "pending" ? "accent" : entry.approvalStatus === "denied" || entry.approvalStatus === "blocked" ? "danger" : "",
  );
  const label = $derived(entry.command ?? entry.path ?? entry.tool);
  /** Output is only built when the row is open (it can be large). */
  const outputText = $derived.by(() => {
    if (!open) return "";
    const out = entry.item.output;
    if (entry.result) return entry.result.output;
    if (out == null) return "";
    return typeof out === "string" ? out : JSON.stringify(out, null, 2);
  });
  const args = $derived.by(() => {
    if (!open || entry.kind === "command") return "";
    const rest = Object.fromEntries(Object.entries(entry.item.input).filter(([k]) => k !== "session_id" && k !== "content"));
    return Object.keys(rest).length ? JSON.stringify(rest, null, 2) : "";
  });
  const note = $derived(entry.result?.note ?? (entry.item.output as { note?: string } | null)?.note ?? null);
</script>

<li class="row" class:failed class:compact>
  <div class="line">
    <time class="time" datetime={new Date(entry.at).toISOString()}>{fmtClock(entry.at)}</time>
    <span class="what">
      {#if entry.kind !== "command"}<span class="tool mono">{entry.tool}</span>{/if}
      {#if label !== entry.tool}<code class="cmd mono" title={label}>{label}</code>{/if}
    </span>
    <span class="pill {tone}">{t(`tool.status.${entry.exec}` as MessageKey)}</span>
  </div>
  <div class="meta">
    <!-- While it waits, the status pill already says so. -->
    {#if entry.approvalStatus !== "none" && entry.exec !== "waiting" && !(entry.exec === "denied" && entry.approvalStatus === "denied")}<span class="pill {apprTone}">{t(`ai.appr.${entry.approvalStatus}`)}</span>{/if}
    {#if entry.risk}<span class:bad={entry.risk !== "read"}>{t(`approval.risk.${entry.risk}`)}</span>{/if}
    {#if entry.exitCode != null}<span class:bad={entry.exitCode !== 0}>{t("tool.exit", { code: entry.exitCode })}</span>{/if}
    {#if entry.durationMs != null && entry.item.done && entry.exec !== "denied"}<span>{fmtMs(entry.durationMs)}</span>{/if}
    {#if entry.result?.truncated}<span title={t("ai.activity.truncated")}>{t("tool.truncated")}</span>{/if}
    <span class="grow"></span>
    <span class="acts">
      {#if entry.kind === "command"}
        <button class="link" onclick={() => jumpToTerminal(sessionId, entry.command)}><Icon name="jump" size={11} />{t("tool.jump")}</button>
      {/if}
      <button class="link" onclick={() => (open = !open)} aria-expanded={open}>
        <Icon name={open ? "chevronDown" : "chevronRight"} size={11} />{open ? t("ai.activity.hideDetails") : t("ai.activity.details")}
      </button>
    </span>
  </div>
  {#if open}
    <dl class="details">
      <dt>{t("ai.activity.agent")}</dt>
      <dd>{entry.actor ?? "—"}</dd>
      <dt>{t("ai.activity.target")}</dt>
      <dd class="target">
        {#if target}
          <span>{target.name}</span>
          {#if target.env === "prod"}<span class="env-prod-badge">{t("ai.prodBadge")}</span>{:else if target.env !== "none"}<span class="dim">{t(`env.${target.env}`)}</span>{/if}
          {#if target.user || target.host}<span class="mono dim">{target.user ?? ""}{target.user && target.host ? "@" : ""}{target.host ?? ""}</span>{/if}
        {:else}—{/if}
      </dd>
      {#if entry.path}<dt>{t("ai.activity.file")}</dt><dd class="mono">{entry.path}</dd>{/if}
      <dt>{t("ai.activity.started")}</dt>
      <dd>{fmtClock(entry.at)}</dd>
      {#if entry.approval?.req.reason}<dt>{t("ai.activity.reason")}</dt><dd>{entry.approval.req.reason}</dd>{/if}
      {#if entry.approval?.decision?.type === "approve_edited"}<dt>{t("approval.edit")}</dt><dd class="mono"><del>{entry.approval.req.command}</del></dd>{/if}
      {#if note}<dt>{t("ai.activity.note")}</dt><dd>{note}</dd>{/if}
      <dt>{t("ai.activity.callId")}</dt>
      <dd class="mono dim">{entry.item.callId}</dd>
    </dl>
    {#if args}<pre class="out mono">{args}</pre>{/if}
    {#if entry.item.done}
      {#if entry.result?.truncated}<p class="trunc">{t("ai.activity.truncated")}</p>{/if}
      <pre class="out mono" class:err={entry.item.isError}>{outputText || t("tool.noOutput")}</pre>
    {/if}
  {/if}
</li>

<style>
  .row {
    list-style: none;
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
    background: var(--panel);
    overflow: hidden;
  }
  .row.failed {
    border-color: color-mix(in srgb, var(--warn) 45%, var(--border));
  }
  .row.compact {
    border: 0;
    border-radius: 0;
    background: none;
  }
  .line {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 10px 2px;
    min-width: 0;
  }
  .time {
    flex: none;
    font-size: var(--fs-small);
    color: var(--faint);
    font-variant-numeric: tabular-nums;
  }
  .what {
    flex: 1;
    min-width: 0;
    display: flex;
    gap: 6px;
    align-items: baseline;
    overflow: hidden;
  }
  .tool {
    flex: none;
    color: var(--muted);
    font-size: var(--fs-small);
  }
  .cmd {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-meta);
    color: var(--text);
  }
  .meta {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px 10px;
    padding: 3px 10px 7px;
    font-size: var(--fs-small);
    color: var(--faint);
    font-variant-numeric: tabular-nums;
  }
  .meta .bad {
    color: var(--warn);
  }
  .grow {
    flex: 1;
  }
  .link {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    color: var(--muted);
    font-size: var(--fs-small);
  }
  .link:hover {
    color: var(--text);
  }
  .details {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 3px 10px;
    margin: 0;
    padding: 8px 10px;
    border-top: 1px solid var(--border);
    font-size: var(--fs-small);
  }
  .details dt {
    color: var(--faint);
  }
  .details dd {
    margin: 0;
    min-width: 0;
    color: var(--text-2);
    overflow-wrap: anywhere;
  }
  .dim {
    color: var(--faint);
  }
  .target {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 2px 6px;
  }
  .acts {
    display: inline-flex;
    gap: 10px;
    margin-left: auto;
    white-space: nowrap;
  }
  .trunc {
    margin: 0;
    padding: 6px 10px;
    border-top: 1px solid var(--border);
    font-size: var(--fs-small);
    color: var(--warn);
  }
  .out {
    margin: 0;
    padding: 8px 10px;
    max-height: 240px;
    overflow: auto;
    font-size: var(--fs-small);
    line-height: 1.5;
    white-space: pre;
    background: var(--bg);
    border-top: 1px solid var(--border);
    color: var(--text-2);
  }
  .out.err {
    color: var(--danger);
    white-space: pre-wrap;
  }
</style>
