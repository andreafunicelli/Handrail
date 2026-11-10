<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";
  import { transfers } from "../../stores/transfers.svelte";
  import { failedCount, ordered, percent, type Transfer } from "../../util/transfers";
  import { formatBytes, relativeTime } from "../../util/time";

  /** Every upload, download and copy of this run of the app (kept in memory only). */
  let { onclose }: { onclose: () => void } = $props();
  let box: HTMLDivElement;

  const items = $derived(ordered(transfers.list));
  const failed = $derived(failedCount(transfers.list));
  const finished = $derived(transfers.list.length - transfers.running);

  const ICONS = { up: "upload", down: "download", copy: "copy" } as const;
  const DIR = { up: "files.transfers.up", down: "files.transfers.down", copy: "files.transfers.copy" } as const;

  function bytes(x: Transfer) {
    if (x.state === "done") return x.total ? formatBytes(x.total) : "";
    if (!x.total) return t("files.transfers.starting");
    return t("files.transfers.bytes", { done: formatBytes(x.done), total: formatBytes(x.total) });
  }

  onMount(() => {
    box.focus();
    const away = (e: PointerEvent) => {
      const el = e.target as Element | null;
      if (!box.contains(el) && !el?.closest?.("[data-transfers-button]")) onclose();
    };
    document.addEventListener("pointerdown", away, true);
    return () => document.removeEventListener("pointerdown", away, true);
  });
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  class="panel"
  role="dialog"
  aria-label={t("files.transfers.title")}
  tabindex="-1"
  bind:this={box}
  onkeydown={(e) => e.key === "Escape" && (e.stopPropagation(), onclose())}
>
  <header>
    <h2>{t("files.transfers.title")}</h2>
    <span class="counts" aria-live="polite">
      {t("files.transfers.counts", { running: transfers.running, failed })}
    </span>
    <span class="grow"></span>
    <button class="btn sm ghost" disabled={!finished} onclick={() => transfers.clearFinished()}>{t("files.clearTransfers")}</button>
    <button class="icon-btn" aria-label={t("common.close")} title={t("common.close")} onclick={onclose}><Icon name="x" size={14} /></button>
  </header>
  <p class="note">{t("files.transfers.note")}</p>
  {#if !items.length}
    <p class="empty">{t("files.transfers.empty")}</p>
  {:else}
    <ul class="list" aria-label={t("files.transfers.title")}>
      {#each items as x (x.id)}
        {@const pct = percent(x)}
        {@const to = x.state === "done" && x.direction === "down" && x.result ? x.result : x.dest}
        <li class="item {x.state}">
          <Icon name={ICONS[x.direction]} size={14} />
          <div class="body">
            <div class="top">
              <span class="name" title={x.name}>{x.name}</span>
              <span class="sr-only">{t(DIR[x.direction])}</span>
              <span class="state">
                {#if x.state === "running"}{t("files.transfers.running")}{:else if x.state === "done"}{t("files.transfers.done")}{:else}{t("files.transfers.failed")}{/if}
              </span>
            </div>
            <div class="route"><span class="k">{t("files.transfers.from")}</span><span class="v" title={x.source}>{x.source}</span></div>
            <div class="route"><span class="k">{t("files.transfers.to")}</span><span class="v" title={to}>{to}</span></div>
            {#if x.state === "running"}
              <div class="prog">
                <progress max={x.total || 1} value={x.total ? x.done : 0} aria-label={t("files.transfers.progressOf", { name: x.name })}></progress>
                <span class="meta">{bytes(x)}{pct != null ? ` · ${pct}%` : ""}</span>
              </div>
            {:else if x.state === "done"}
              <span class="meta">{[bytes(x), x.ended ? relativeTime(x.ended) : ""].filter(Boolean).join(" · ")}</span>
            {:else}
              <span class="err">{t("files.transfers.error", { error: x.result ?? "" })}</span>
              {#if x.total}<span class="meta">{t("files.transfers.stoppedAt", { done: formatBytes(x.done), total: formatBytes(x.total) })}</span>{/if}
            {/if}
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .panel {
    position: fixed;
    left: calc(var(--rail-w) + 8px);
    bottom: 12px;
    z-index: 55;
    display: flex;
    flex-direction: column;
    width: min(440px, calc(100vw - var(--rail-w) - 24px));
    max-height: min(560px, calc(100vh - 24px));
    border: 1px solid var(--border-strong);
    border-radius: var(--r-lg);
    background: var(--panel);
    box-shadow: var(--shadow-float);
    outline: none;
  }
  header {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    padding: 10px 10px 6px 14px;
  }
  h2 {
    margin: 0;
    font-size: var(--fs);
    font-weight: 600;
  }
  .counts {
    color: var(--faint);
    font-size: var(--fs-small);
    white-space: nowrap;
  }
  .grow {
    flex: 1;
  }
  .note {
    margin: 0;
    padding: 0 14px 8px;
    color: var(--faint);
    font-size: var(--fs-small);
    border-bottom: 1px solid var(--border);
  }
  .empty {
    margin: 0;
    padding: 18px 14px;
    color: var(--muted);
    font-size: var(--fs-meta);
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 4px 0;
    overflow: auto;
  }
  .item {
    display: flex;
    gap: 10px;
    padding: 8px 14px;
    font-size: var(--fs-meta);
  }
  .item + .item {
    border-top: 1px solid var(--border);
  }
  .item > :global(svg) {
    flex: none;
    margin-top: 2px;
    color: var(--faint);
  }
  .body {
    display: grid;
    gap: 3px;
    flex: 1;
    min-width: 0;
  }
  .top {
    display: flex;
    gap: 8px;
    align-items: baseline;
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
    font-weight: 500;
  }
  .state {
    flex: none;
    font-size: var(--fs-small);
    color: var(--muted);
  }
  .done .state {
    color: var(--ok);
  }
  .failed .state,
  .failed > :global(svg) {
    color: var(--danger);
  }
  .route {
    display: flex;
    gap: 6px;
    min-width: 0;
    color: var(--faint);
    font-size: var(--fs-small);
  }
  .route .k {
    flex: none;
    width: 2.6em;
    color: var(--faint);
  }
  .route .v {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--muted);
  }
  .prog {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  progress {
    flex: 1;
    height: 6px;
  }
  .meta {
    color: var(--faint);
    font-size: var(--fs-small);
    white-space: nowrap;
  }
  .err {
    color: var(--danger);
    font-size: var(--fs-small);
    overflow-wrap: anywhere;
  }
</style>
