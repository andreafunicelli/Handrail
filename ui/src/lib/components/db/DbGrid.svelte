<script lang="ts">
  import type { DbResultSet, DbValue } from "../../api/types";
  import { t } from "../../i18n";
  import { copyText } from "../../util/clipboard";
  import { cellText, columnWidths } from "../../util/db";

  /** A result set as a grid: only the rows in view are in the page, so large results scroll
   *  smoothly. Click a cell to select it, Ctrl/⌘+C copies it. */
  let { result }: { result: DbResultSet } = $props();

  const ROW = 26;
  const OVERSCAN = 8;
  let scroller = $state<HTMLDivElement>();
  let scrollTop = $state(0);
  let height = $state(400);
  let selected = $state<{ r: number; c: number } | null>(null);

  const widths = $derived(columnWidths(result));
  const total = $derived(widths.reduce((a, b) => a + b, 0) + 48);
  const first = $derived(Math.max(0, Math.floor(scrollTop / ROW) - OVERSCAN));
  const last = $derived(Math.min(result.rows.length, Math.ceil((scrollTop + height) / ROW) + OVERSCAN));
  const visible = $derived(result.rows.slice(first, last));

  $effect(() => {
    void result;
    selected = null;
    if (scroller) scroller.scrollTop = 0;
  });

  function onkeydown(e: KeyboardEvent) {
    if (!selected) return;
    const mod = e.metaKey || e.ctrlKey;
    if (mod && e.key.toLowerCase() === "c") {
      e.preventDefault();
      void copyText(cellText(result.rows[selected.r]?.[selected.c] ?? null));
      return;
    }
    const moves: Record<string, [number, number]> = { ArrowUp: [-1, 0], ArrowDown: [1, 0], ArrowLeft: [0, -1], ArrowRight: [0, 1] };
    const m = moves[e.key];
    if (!m) return;
    e.preventDefault();
    const r = Math.min(result.rows.length - 1, Math.max(0, selected.r + m[0]));
    const c = Math.min(result.columns.length - 1, Math.max(0, selected.c + m[1]));
    selected = { r, c };
    const top = r * ROW;
    if (scroller && (top < scroller.scrollTop || top + ROW > scroller.scrollTop + height - ROW)) scroller.scrollTop = Math.max(0, top - height / 2);
  }

  const kind = (v: DbValue) => (v === null ? "null" : typeof v === "string" ? "text" : "bytes");
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div
  class="grid"
  role="grid"
  aria-rowcount={result.rows.length}
  tabindex="0"
  bind:this={scroller}
  bind:clientHeight={height}
  onscroll={(e) => (scrollTop = e.currentTarget.scrollTop)}
  {onkeydown}
>
  <div class="inner" style:width="{total}px" style:height="{(result.rows.length + 1) * ROW}px">
    <div class="row head" role="row" style:top="0px">
      <span class="num"></span>
      {#each result.columns as c, i (i)}
        <span class="cell" role="columnheader" style:width="{widths[i]}px" title={`${c.name} · ${c.type_name}`}>
          <span class="name">{c.name}</span><span class="type">{c.type_name.toLowerCase()}</span>
        </span>
      {/each}
    </div>
    {#each visible as row, k (first + k)}
      {@const r = first + k}
      <div class="row" role="row" style:top="{(r + 1) * ROW}px">
        <span class="num">{r + 1}</span>
        {#each row as v, c (c)}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <span
            class="cell {kind(v)}"
            role="gridcell"
            tabindex="-1"
            class:sel={selected?.r === r && selected?.c === c}
            style:width="{widths[c]}px"
            title={v === null ? "NULL" : cellText(v).slice(0, 2000)}
            onclick={() => (selected = { r, c })}>{v === null ? "NULL" : cellText(v).slice(0, 500)}</span
          >
        {/each}
      </div>
    {/each}
  </div>
  {#if !result.rows.length}
    <p class="empty">{t("db.noRows")}</p>
  {/if}
</div>

<style>
  .grid {
    position: relative;
    height: 100%;
    overflow: auto;
    outline: none;
    font-family: var(--font-mono);
    font-size: 12px;
  }
  .inner {
    position: relative;
    min-width: 100%;
  }
  .row {
    position: absolute;
    left: 0;
    right: 0;
    display: flex;
    height: 26px;
    border-bottom: 1px solid var(--border);
  }
  .row:hover:not(.head) {
    background: var(--hover);
  }
  .head {
    position: sticky;
    z-index: 1;
    background: var(--panel);
    font-family: var(--font-ui, inherit);
    font-weight: 600;
  }
  .num {
    flex: 0 0 48px;
    padding: 0 8px;
    line-height: 26px;
    text-align: right;
    color: var(--faint);
    background: var(--panel);
    border-right: 1px solid var(--border);
    position: sticky;
    left: 0;
  }
  .cell {
    flex: none;
    padding: 0 8px;
    line-height: 26px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    border-right: 1px solid var(--border);
    cursor: default;
  }
  .head .cell {
    display: flex;
    gap: 6px;
    align-items: baseline;
  }
  .head .name {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .head .type {
    color: var(--faint);
    font-weight: 400;
    font-size: 11px;
  }
  .cell.null {
    color: var(--faint);
    font-style: italic;
  }
  .cell.bytes {
    color: var(--syn-number, var(--muted));
  }
  .cell.sel {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
    background: var(--accent-soft);
  }
  .empty {
    position: absolute;
    top: 40px;
    left: 0;
    right: 0;
    text-align: center;
    color: var(--muted);
    margin: 0;
  }
</style>
