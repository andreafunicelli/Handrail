<script lang="ts">
  import { segmented } from "../../actions/segmented";
  import Icon from "../icons/Icon.svelte";
  import ActivityRow from "./ActivityRow.svelte";
  import ApprovalCard from "./ApprovalCard.svelte";
  import { t } from "../../i18n";
  import type { Chat } from "../../stores/agent.svelte";
  import { buildActivity, filterActivity, isFailedExec, type ActivityFilter, type ToolKind } from "../../util/agentActivity";
  import { fmtClock, type Target } from "./target";

  let { chat, sessionId, target }: { chat: Chat | undefined; sessionId: string; target: Target | null } = $props();

  let status = $state<ActivityFilter>("all");
  let kind = $state<ToolKind | "all">("all");

  // Built from tool, approval, user, error and limit items only: text deltas do not re-run it.
  const entries = $derived(buildActivity(chat?.items ?? []));
  const shown = $derived(filterActivity(entries, status, kind));
  const pending = $derived((chat?.items ?? []).filter((i) => i.kind === "approval" && !i.decision));
  const failedCount = $derived(
    entries.reduce((n, e) => n + (e.type === "tool" ? +isFailedExec(e.exec) : e.type === "group" ? e.entries.filter((x) => isFailedExec(x.exec)).length : e.type === "error" ? 1 : 0), 0),
  );
  const kinds: (ToolKind | "all")[] = ["all", "command", "file", "db", "other"];
</script>

<div class="activity">
  {#if pending.length}
    <section class="pinned" aria-label={t("ai.activity.pinned")}>
      <h3><Icon name="shield" size={12} />{t("ai.activity.pinned")}</h3>
      {#each pending as item (item.id)}
        {#if item.kind === "approval"}<ApprovalCard {item} {sessionId} />{/if}
      {/each}
    </section>
  {/if}

  {#if entries.length}
    <div class="filters">
      <div class="seg" role="group" aria-label={t("ai.activity.filter")} use:segmented>
        <button aria-pressed={status === "all"} onclick={() => (status = "all")}>{t("ai.activity.filter.all")}</button>
        <button aria-pressed={status === "failed"} onclick={() => (status = "failed")}>
          {t("ai.activity.filter.failed")}{#if failedCount}<span class="n">{failedCount}</span>{/if}
        </button>
        <button aria-pressed={status === "pending"} onclick={() => (status = "pending")}>
          {t("ai.activity.filter.pending")}{#if pending.length}<span class="n hot">{pending.length}</span>{/if}
        </button>
      </div>
      <select class="select" bind:value={kind} aria-label={t("ai.activity.kind")}>
        {#each kinds as k (k)}<option value={k}>{t(`ai.activity.kind.${k}`)}</option>{/each}
      </select>
    </div>

    <ol class="timeline">
      {#each shown as e (e.id)}
        {#if e.type === "tool"}
          <ActivityRow entry={e} {sessionId} {target} />
        {:else if e.type === "group"}
          <li class="group">
            <details>
              <summary>
                <time class="time">{fmtClock(e.at)}</time>
                <span class="mono gname">{t("ai.activity.group", { tool: e.tool, n: e.entries.length })}</span>
                {#if e.entries.some((x) => isFailedExec(x.exec))}<span class="pill warn">{t("tool.status.error")}</span>{/if}
              </summary>
              <ol class="sub">
                {#each e.entries as x (x.id)}<ActivityRow entry={x} {sessionId} {target} compact />{/each}
              </ol>
            </details>
          </li>
        {:else if e.type === "prompt"}
          <li class="prompt">
            <time class="time">{fmtClock(e.at)}</time>
            <span class="plabel">{t("ai.activity.prompt")}</span>
            <span class="ptext" title={e.text}>{e.text}</span>
          </li>
        {:else if e.type === "approval"}
          <!-- Pending ones are pinned above. -->
          {#if e.item.decision}<li class="orphan"><ApprovalCard item={e.item} {sessionId} /></li>{/if}
        {:else if e.type === "error"}
          <li class="error" role="alert">
            <time class="time">{fmtClock(e.at)}</time>
            <Icon name="alert" size={12} /><span>{e.message}</span>
          </li>
        {:else if e.type === "limit"}
          <li class="limit"><time class="time">{fmtClock(e.at)}</time><span>{t("ai.activity.limit", { n: e.steps })}</span></li>
        {/if}
      {:else}
        <li class="none">{t("ai.activity.noMatch")}</li>
      {/each}
    </ol>
  {:else if !pending.length}
    <p class="empty">{t("ai.activity.empty")}</p>
  {/if}
</div>

<style>
  .activity {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .pinned {
    display: grid;
    gap: 8px;
    padding-bottom: 10px;
    border-bottom: 1px solid var(--border);
  }
  h3 {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    font-size: var(--fs-small);
    font-weight: 600;
    color: var(--accent-text);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .filters {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .filters .select {
    margin-left: auto;
    width: auto;
    height: var(--ctl-h-sm);
    font-size: var(--fs-meta);
  }
  .n {
    margin-left: 5px;
    font-size: var(--fs-tiny);
    color: var(--faint);
    font-variant-numeric: tabular-nums;
  }
  .n.hot {
    color: var(--accent-text);
    font-weight: 650;
  }
  .timeline,
  .sub {
    display: grid;
    gap: 6px;
    margin: 0;
    padding: 0;
  }
  .sub {
    gap: 0;
    border-top: 1px solid var(--border);
  }
  .sub > :global(li + li) {
    border-top: 1px solid var(--border);
  }
  li {
    list-style: none;
  }
  .time {
    flex: none;
    font-size: var(--fs-small);
    color: var(--faint);
    font-variant-numeric: tabular-nums;
  }
  .group details {
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
    background: var(--panel);
    overflow: hidden;
  }
  .group summary {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 10px;
    cursor: pointer;
    font-size: var(--fs-meta);
    color: var(--muted);
  }
  .group summary:hover {
    color: var(--text);
  }
  .gname {
    flex: 1;
    font-size: var(--fs-small);
  }
  .prompt {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
    padding: 6px 2px 2px;
    font-size: var(--fs-meta);
  }
  .plabel {
    flex: none;
    color: var(--faint);
    font-size: var(--fs-small);
  }
  .ptext {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
    font-weight: 550;
  }
  .error {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 7px 10px;
    border-radius: var(--r-lg);
    background: var(--danger-soft);
    color: var(--danger);
    font-size: var(--fs-meta);
  }
  .error .time {
    color: inherit;
    opacity: 0.8;
  }
  .limit {
    display: flex;
    gap: 8px;
    padding: 6px 10px;
    border-radius: var(--r-lg);
    background: var(--hover);
    color: var(--muted);
    font-size: var(--fs-meta);
  }
  .none,
  .empty {
    margin: 0;
    padding: 8px 2px;
    color: var(--muted);
    font-size: var(--fs-meta);
  }
</style>
