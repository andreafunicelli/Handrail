<script lang="ts">
  import { tick } from "svelte";
  import type { DbResultSet, DbValue } from "../../api/types";
  import { t } from "../../i18n";
  import { copyText } from "../../util/clipboard";
  import { cellText, columnWidths, editedValue, type GridEdits } from "../../util/db";

  /** A result set as a grid: only the rows in view are in the page, so large results scroll
   *  smoothly. Click a cell to select it, Ctrl/⌘+C copies it. With `edits` (one table's rows),
   *  a double click or Enter edits a cell; changes stay marked until they are saved. */
  let {
    result,
    edits = $bindable(null),
    selected = $bindable(null),
    locked = false,
    sort = null,
    onsort,
    firstRow = 0,
  }: {
    result: DbResultSet;
    edits?: GridEdits | null;
    selected?: { r: number; c: number } | null;
    /** Edits are being saved: nothing more can be changed until they are. */
    locked?: boolean;
    /** The column the rows are sorted by (a table's rows); with `onsort`, headers sort. */
    sort?: { column: string | null; desc: boolean } | null;
    onsort?: (column: string) => void;
    /** Rows before this page (the numbers in the first column go on from there). */
    firstRow?: number;
  } = $props();

  const ROW = 26;
  const OVERSCAN = 8;
  let scroller = $state<HTMLDivElement>();
  let scrollTop = $state(0);
  let height = $state(400);
  let editing = $state<{ r: number; c: number; text: string; start: string; touched: boolean } | null>(null);
  let editInput = $state<HTMLInputElement>();

  const widths = $derived(columnWidths(result));
  const total = $derived(widths.reduce((a, b) => a + b, 0) + 48);
  const first = $derived(Math.max(0, Math.floor(scrollTop / ROW) - OVERSCAN));
  const count = $derived(result.rows.length + (edits?.added.length ?? 0));
  const last = $derived(Math.min(count, Math.ceil((scrollTop + height) / ROW) + OVERSCAN));
  const visible = $derived(Array.from({ length: Math.max(0, last - first) }, (_, k) => first + k));

  $effect(() => {
    void result;
    selected = null;
    editing = null;
    if (scroller) scroller.scrollTop = 0;
  });

  /** The value shown in a cell: the edited one, or `undefined` for a new row's default. */
  const value = (r: number, c: number): DbValue | undefined =>
    edits ? editedValue(result, edits, r, c) : result.rows[r]?.[c];
  const changed = (r: number, c: number) =>
    !!edits && (r >= result.rows.length ? c in (edits.added[r - result.rows.length] ?? {}) : c in (edits.cells[r] ?? {}));

  async function startEdit(r: number, c: number) {
    if (!edits || locked || edits.deleted[r]) return;
    const v = value(r, c);
    // Binary values are shown as hex and cannot be typed back.
    if (v !== null && v !== undefined && typeof v !== "string") return;
    selected = { r, c };
    // An editor closed untouched changes nothing (NULL stays NULL, a new row's DEFAULT stays);
    // typing and clearing it again is a deliberate empty string.
    editing = { r, c, text: v ?? "", start: v ?? "", touched: false };
    await tick();
    editInput?.focus();
    editInput?.select();
  }

  function commit() {
    if (!editing || !edits) return;
    const { r, c, text, start, touched } = editing;
    editing = null;
    if (touched || text !== start) setCell(r, c, text);
    scroller?.focus();
  }

  /** Set a cell of the edited rows; going back to the original value drops the change. */
  export function setCell(r: number, c: number, v: string | null) {
    if (!edits || locked || edits.deleted[r]) return;
    if (r >= result.rows.length) {
      edits.added[r - result.rows.length][c] = v;
      return;
    }
    const row = (edits.cells[r] ??= {});
    if (result.rows[r][c] === v) delete row[c];
    else row[c] = v;
    if (!Object.keys(row).length) delete edits.cells[r];
  }

  /** Bring a row into view (a row just added at the end). */
  export function reveal(r: number) {
    const top = (r + 1) * ROW;
    if (scroller && (top < scroller.scrollTop + ROW || top + ROW > scroller.scrollTop + height)) scroller.scrollTop = Math.max(0, top - height / 2);
  }

  function onkeydown(e: KeyboardEvent) {
    if (!selected || editing) return;
    const mod = e.metaKey || e.ctrlKey;
    if (mod && e.key.toLowerCase() === "c") {
      e.preventDefault();
      void copyText(cellText(value(selected.r, selected.c) ?? null));
      return;
    }
    if (edits && (e.key === "Enter" || e.key === "F2")) {
      e.preventDefault();
      void startEdit(selected.r, selected.c);
      return;
    }
    const moves: Record<string, [number, number]> = { ArrowUp: [-1, 0], ArrowDown: [1, 0], ArrowLeft: [0, -1], ArrowRight: [0, 1] };
    const m = moves[e.key];
    if (!m) return;
    e.preventDefault();
    const r = Math.min(count - 1, Math.max(0, selected.r + m[0]));
    const c = Math.min(result.columns.length - 1, Math.max(0, selected.c + m[1]));
    selected = { r, c };
    const top = r * ROW;
    if (scroller && (top < scroller.scrollTop || top + ROW > scroller.scrollTop + height - ROW)) scroller.scrollTop = Math.max(0, top - height / 2);
  }

  const kind = (v: DbValue | undefined) => (v === undefined ? "default" : v === null ? "null" : typeof v === "string" ? "text" : "bytes");
  const shown = (v: DbValue | undefined, n: number) => (v === undefined ? t("db.default") : v === null ? "NULL" : cellText(v).slice(0, n));
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div
  class="grid"
  role="grid"
  aria-rowcount={count}
  tabindex="0"
  bind:this={scroller}
  bind:clientHeight={height}
  onscroll={(e) => (scrollTop = e.currentTarget.scrollTop)}
  {onkeydown}
>
  <div class="inner" style:width="{total}px" style:height="{(count + 1) * ROW}px">
    <div class="row head" role="row" style:top="0px">
      <span class="num"></span>
      {#each result.columns as c, i (i)}
        {@const sorted = sort?.column === c.name}
        <span
          class="cell"
          class:sortable={!!onsort}
          role="columnheader"
          aria-sort={sorted ? (sort?.desc ? "descending" : "ascending") : "none"}
          style:width="{widths[i]}px"
          title={onsort ? t("db.sortHint", { name: c.name, type: c.type_name }) : `${c.name} · ${c.type_name}`}
          onclick={() => onsort?.(c.name)}
          onkeydown={(e) => e.key === "Enter" && onsort?.(c.name)}
          tabindex={onsort ? 0 : undefined}
        >
          <span class="name">{c.name}</span><span class="type">{c.type_name.toLowerCase()}</span>
          {#if sorted}<span class="arrow" aria-hidden="true">{sort?.desc ? "▼" : "▲"}</span>{/if}
        </span>
      {/each}
    </div>
    {#each visible as r (r)}
      {@const added = r >= result.rows.length}
      <div class="row" role="row" style:top="{(r + 1) * ROW}px" class:deleted={!!edits?.deleted[r]} class:added>
        <span class="num">{added ? "+" : firstRow + r + 1}</span>
        {#each result.columns as _, c (c)}
          {@const v = value(r, c)}
          {#if editing?.r === r && editing?.c === c}
            <input
              class="cell edit"
              style:width="{widths[c]}px"
              bind:this={editInput}
              bind:value={editing.text}
              oninput={() => editing && (editing.touched = true)}
              aria-label={result.columns[c].name}
              onkeydown={(e) => {
                if (e.key === "Enter") (e.preventDefault(), commit());
                else if (e.key === "Escape") (e.preventDefault(), (editing = null), scroller?.focus());
                e.stopPropagation();
              }}
              onblur={commit}
            />
          {:else}
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <span
              class="cell {kind(v)}"
              role="gridcell"
              tabindex="-1"
              class:sel={selected?.r === r && selected?.c === c}
              class:changed={changed(r, c)}
              style:width="{widths[c]}px"
              title={shown(v, 2000)}
              onclick={() => (selected = { r, c })}
              ondblclick={() => startEdit(r, c)}>{shown(v, 500)}</span
            >
          {/if}
        {/each}
      </div>
    {/each}
  </div>
  {#if !count}
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
    font-size: var(--fs-meta);
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
  .head .sortable {
    cursor: pointer;
  }
  .head .sortable:hover {
    background: var(--hover);
  }
  .head .arrow {
    margin-left: auto;
    color: var(--accent);
    font-size: var(--fs-tiny);
  }
  .head .type {
    color: var(--faint);
    font-weight: 400;
    font-size: var(--fs-small);
  }
  .cell.bytes {
    color: var(--syn-number, var(--muted));
  }
  .cell.null,
  .cell.default {
    color: var(--faint);
    font-style: italic;
  }
  .cell.changed {
    background: var(--warn-soft, rgba(234, 179, 8, 0.16));
  }
  .row.added .num {
    color: var(--accent);
  }
  .row.deleted .cell {
    text-decoration: line-through;
    color: var(--faint);
    background: var(--danger-soft);
  }
  input.cell.edit {
    border: 0;
    outline: 2px solid var(--accent);
    outline-offset: -2px;
    background: var(--bg);
    color: var(--text);
    font: inherit;
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
