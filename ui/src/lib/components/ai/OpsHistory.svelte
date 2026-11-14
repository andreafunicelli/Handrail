<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "../icons/Icon.svelte";
  import { fmtDateTime, fmtNumber, t, type MessageKey } from "../../i18n";
  import { opsLog } from "../../stores/opsLog.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { ui } from "../../stores/ui.svelte";
  import { filterEntries, isFailure, type OpsDecision, type OpsFilter, type OpsLogSettings } from "../../util/opslog";
  import { fmtMs } from "./target";

  const RETENTION = [7, 30, 90, 365];
  const CAPS = [500, 1000, 5000];
  const DECISIONS: OpsDecision[] = ["approved", "edited", "denied", "auto", "blocked", "none"];
  const PAGE = 200;

  let filter = $state<OpsFilter>({ connection: "", decision: "", outcome: "", text: "" });
  let limit = $state(PAGE);
  let busy = $state(false);

  onMount(() => {
    // Write what is queued first, so the list is complete; then read the vault.
    void opsLog.flush().finally(() => opsLog.load());
  });

  const all = $derived(opsLog.all);
  const connectionsSeen = $derived.by(() => {
    const m = new Map<string, string>();
    for (const e of all) m.set(e.connection_id ?? "local", e.connection_id ? (e.connection_name ?? e.connection_id) : t("ops.filter.local"));
    return [...m].sort((a, b) => a[1].localeCompare(b[1]));
  });
  // Newest first.
  const shown = $derived(filterEntries(all, filter).reverse());

  const STATES = new Set(["completed", "timed_out", "interrupted", "needs_user", "denied", "error", "failed", "rejected", "no_result"]);
  function outcomeLabel(o: string): string {
    if (o === "ok") return t("tool.status.completed");
    if (o === "saved") return t("ai.summary.file.saved");
    return STATES.has(o) ? t(`tool.status.${o}` as MessageKey) : o;
  }

  async function update(patch: Partial<OpsLogSettings>) {
    try {
      await opsLog.saveSettings({ ...$state.snapshot(opsLog.settings), ...patch });
      toasts.success(t("ops.saved"));
    } catch (e) {
      toasts.error(e);
    }
  }

  async function clear() {
    const ok = await ui.ask(t("ops.clearConfirm"), { body: t("ops.clearBody"), confirmLabel: t("ops.clear"), danger: true });
    if (!ok) return;
    busy = true;
    try {
      await opsLog.clear();
      toasts.success(t("ops.cleared"));
    } catch (e) {
      toasts.error(e);
    } finally {
      busy = false;
    }
  }
</script>

<h3>{t("ops.title")}</h3>
<p class="hint sub">{t("ops.body")}</p>

{#if opsLog.error && !opsLog.loaded}
  <p class="hint err" role="alert">{t("ops.unavailable", { error: opsLog.error })}</p>
{:else}
  <div class="card opts">
    <label class="check">
      <input type="checkbox" checked={opsLog.settings.enabled} onchange={(e) => update({ enabled: e.currentTarget.checked })} />
      <span>{t("ops.enabled")}</span>
    </label>
    <label class="check">
      <input type="checkbox" checked={opsLog.settings.record_commands} disabled={!opsLog.settings.enabled} onchange={(e) => update({ record_commands: e.currentTarget.checked })} />
      <span>{t("ops.recordCommands")}</span>
    </label>
    <p class="hint indent">{t("ops.recordCommandsHint")}</p>
    <div class="row2">
      <label>
        <span class="lbl">{t("ops.retention")}</span>
        <select class="select" value={opsLog.settings.retention_days} onchange={(e) => update({ retention_days: Number(e.currentTarget.value) })}>
          {#each RETENTION as d (d)}<option value={d}>{t("ops.retentionDays", { n: d })}</option>{/each}
        </select>
      </label>
      <label>
        <span class="lbl">{t("ops.maxEntries")}</span>
        <select class="select" value={opsLog.settings.max_entries} onchange={(e) => update({ max_entries: Number(e.currentTarget.value) })}>
          {#each CAPS as c (c)}<option value={c}>{fmtNumber(c)}</option>{/each}
        </select>
      </label>
      <span class="grow"></span>
      <button class="btn sm danger" disabled={busy || !all.length} onclick={clear}><Icon name="trash" size={12} />{t("ops.clear")}</button>
    </div>
  </div>

  <div class="filters">
    <input class="input" type="search" placeholder={t("ops.filter.text")} aria-label={t("ops.filter.text")} bind:value={filter.text} oninput={() => (limit = PAGE)} />
    <select class="select" bind:value={filter.connection} aria-label={t("ops.filter.connection")}>
      <option value="">{t("ops.filter.allConnections")}</option>
      {#each connectionsSeen as [id, name] (id)}<option value={id}>{name}</option>{/each}
    </select>
    <select class="select" bind:value={filter.decision} aria-label={t("ops.filter.decision")}>
      <option value="">{t("ops.filter.allDecisions")}</option>
      {#each DECISIONS as d (d)}<option value={d}>{t(`ai.appr.${d}`)}</option>{/each}
    </select>
    <select class="select" bind:value={filter.outcome} aria-label={t("ops.filter.outcome")}>
      <option value="">{t("ops.filter.allOutcomes")}</option>
      <option value="ok">{t("ops.filter.ok")}</option>
      <option value="failed">{t("ops.filter.failed")}</option>
    </select>
    <button class="icon-btn" title={t("ops.refresh")} aria-label={t("ops.refresh")} onclick={() => opsLog.flush().finally(() => opsLog.load())}><Icon name="refresh" size={13} /></button>
  </div>
  <p class="hint counts">
    {t("ops.shown", { shown: fmtNumber(shown.length), total: fmtNumber(all.length) })}
    {#if opsLog.pending.length}· {t("ops.unsaved", { n: opsLog.pending.length })}{/if}
    · {t("ops.deviceOnly")}
  </p>

  <div class="card list" role="list">
    {#each shown.slice(0, limit) as e, i (`${e.at}-${e.session_id}-${e.tool}-${e.operation}-${i}`)}
      <div class="entry" role="listitem" class:failed={isFailure(e)}>
        <div class="l1">
          <time datetime={new Date(e.at).toISOString()}>{fmtDateTime(e.at, { dateStyle: "short", timeStyle: "medium" })}</time>
          <span class="conn">{e.connection_name ?? t("ops.filter.local")}</span>
          {#if e.env === "prod"}<span class="env-prod-badge">{t("ai.prodBadge")}</span>{/if}
          <span class="grow"></span>
          <span class="pill {e.decision === 'denied' || e.decision === 'blocked' ? 'danger' : ''}">{t(`ai.appr.${e.decision}`)}</span>
        </div>
        <div class="l2">
          {#if e.tool !== "run_command"}<span class="tool mono">{e.tool}</span>{/if}
          {#if e.operation != null}<code class="mono op">{e.operation}</code>{:else}<span class="dim">{t("ops.notRecorded")}</span>{/if}
        </div>
        <div class="l3">
          {#if e.agent}<span>{e.agent}</span>{/if}
          <span>{outcomeLabel(e.outcome)}</span>
          {#if e.exit_code != null}<span class:bad={e.exit_code !== 0}>{t("tool.exit", { code: e.exit_code })}</span>{/if}
          {#if e.duration_ms != null && e.outcome !== "denied"}<span>{fmtMs(e.duration_ms)}</span>{/if}
          {#if e.error && e.error !== e.outcome}<span class="bad">{t(`ops.err.${e.error}`)}</span>{/if}
          {#if e.risk}<span>{t(`approval.risk.${e.risk}`)}</span>{/if}
        </div>
      </div>
    {:else}
      <p class="hint none">{all.length ? t("ops.noMatch") : t("ops.empty")}</p>
    {/each}
  </div>
  {#if shown.length > limit}
    <button class="btn sm more" onclick={() => (limit += PAGE)}>{t("ops.more", { n: shown.length - limit })}</button>
  {/if}
{/if}

<style>
  .sub {
    margin: -4px 0 10px;
  }
  .err {
    color: var(--danger);
  }
  .opts {
    display: grid;
    gap: 6px;
    padding: 10px 12px;
  }
  .indent {
    margin: -2px 0 4px 24px;
  }
  .row2 {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 12px;
    margin-top: 4px;
  }
  .row2 label {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .row2 .select {
    width: auto;
  }
  .lbl {
    font-size: var(--fs-meta);
    color: var(--muted);
    white-space: nowrap;
  }
  .grow {
    flex: 1;
  }
  .filters {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 14px;
  }
  .filters .input {
    flex: 1 1 100%;
    min-width: 0;
  }
  .filters .select {
    flex: 1 1 0;
    width: auto;
    min-width: 0;
  }
  .counts {
    margin: 6px 0;
  }
  .list {
    max-height: 480px;
    overflow: auto;
  }
  .entry {
    display: grid;
    gap: 3px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
  }
  .entry:last-child {
    border-bottom: 0;
  }
  .entry.failed {
    box-shadow: inset 2px 0 0 var(--warn);
  }
  .l1,
  .l3 {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px 10px;
    font-size: var(--fs-small);
    color: var(--faint);
    font-variant-numeric: tabular-nums;
  }
  .conn {
    color: var(--text-2);
    font-weight: 550;
  }
  .l2 {
    display: flex;
    gap: 8px;
    align-items: baseline;
    min-width: 0;
  }
  .tool {
    flex: none;
    font-size: var(--fs-small);
    color: var(--muted);
  }
  .op {
    min-width: 0;
    font-size: var(--fs-meta);
    color: var(--text);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .dim {
    color: var(--faint);
    font-size: var(--fs-small);
    font-style: italic;
  }
  .bad {
    color: var(--warn);
  }
  .none {
    margin: 0;
    padding: 12px;
  }
  .more {
    margin-top: 8px;
  }
</style>
