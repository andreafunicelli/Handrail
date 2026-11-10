<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import Icon from "../icons/Icon.svelte";
  import DbGrid from "./DbGrid.svelte";
  import { api } from "../../api";
  import type { DbBackupFormat, DbBackupIndex, DbBackupPage, DbResultSet } from "../../api/types";
  import { errorMessage } from "../../api/errors";
  import { t } from "../../i18n";
  import { backups } from "../../stores/backups.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { formatBytes } from "../../util/time";

  /** An SQL backup (mysqldump, pg_dump plain, our export; .sql or .sql.gz) read without a
   *  server: its tables, their structure and rows, and a table saved as CSV, JSON or SQL. */
  let { leaf, onclose }: { leaf: string; onclose: () => void } = $props();

  const PAGE = 500;
  let phase = $state<"picking" | "indexing" | "ready" | "failed">("picking");
  let error = $state<string | null>(null);
  let token = $state<string | null>(null);
  let file = $state<{ name: string; size: number; compressed: boolean } | null>(null);
  let index = $state<DbBackupIndex | null>(null);
  let readBytes = $state(0);
  let transferId = "";

  let selected = $state<number | null>(null);
  let view = $state<"data" | "structure">("data");
  let nameFilter = $state("");
  let filter = $state("");
  let applied = $state("");
  let page = $state<DbBackupPage | null>(null);
  /** Where each page shown so far started (Back pops). */
  let starts = $state<number[]>([0]);
  let loading = $state(false);
  let pageError = $state<string | null>(null);
  let saving = $state<{ format: DbBackupFormat; rows: number } | null>(null);
  let requests = 0;
  let unlisten: (() => void) | null = null;

  const tables = $derived(
    (index?.tables ?? [])
      .map((tb, i) => ({ tb, i }))
      .filter(({ tb }) => !nameFilter.trim() || tb.name.toLowerCase().includes(nameFilter.trim().toLowerCase())),
  );
  const table = $derived(selected !== null ? (index?.tables[selected] ?? null) : null);
  const result = $derived.by((): DbResultSet | null =>
    page ? { columns: page.columns.map((name) => ({ name, type_name: "" })), rows: page.rows, rows_affected: 0, truncated: false } : null,
  );

  function newTransfer() {
    transferId = `bk-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
    return transferId;
  }

  async function listen() {
    unlisten?.();
    unlisten = await api.on("db://transfer", (ev) => {
      if (ev.transfer_id !== transferId) return;
      if (phase === "indexing") readBytes = ev.progress.bytes;
      else if (saving) saving = { ...saving, rows: ev.progress.rows };
    });
  }

  async function open() {
    phase = "picking";
    error = null;
    await listen();
    const id = newTransfer();
    try {
      phase = "indexing";
      readBytes = 0;
      const r = await api.dbBackupOpen(null, id);
      if (!r) {
        onclose();
        return;
      }
      token = r.token;
      file = { name: r.name, size: r.size, compressed: r.compressed };
      index = r.index;
      backups.names[leaf] = r.name;
      phase = "ready";
      const first = r.index.tables.findIndex((x) => !x.view && x.rows > 0);
      if (first >= 0) pick(first);
      else if (r.index.tables.length) pick(0);
    } catch (e) {
      error = errorMessage(e);
      phase = "failed";
    }
  }

  async function reindex(kind: "mysql" | "postgres") {
    if (!token || kind === index?.kind) return;
    phase = "indexing";
    readBytes = 0;
    try {
      index = await api.dbBackupReindex(token, kind, newTransfer());
      phase = "ready";
      page = null;
      selected = null;
      if (index.tables.length) pick(0);
    } catch (e) {
      error = errorMessage(e);
      phase = "failed";
    }
  }

  function pick(i: number) {
    selected = i;
    filter = "";
    applied = "";
    starts = [0];
    page = null;
    pageError = null;
    view = index?.tables[i]?.view ? "structure" : "data";
    if (view === "data") void load();
  }

  async function load() {
    if (!token || selected === null) return;
    const n = ++requests;
    const at = selected;
    loading = true;
    pageError = null;
    try {
      const p = await api.dbBackupRows(token, at, starts[starts.length - 1], PAGE, applied || null);
      if (n !== requests || at !== selected) return;
      page = p;
    } catch (e) {
      if (n !== requests) return;
      pageError = errorMessage(e);
      page = null;
    } finally {
      if (n === requests) loading = false;
    }
  }

  function applyFilter(e: Event) {
    e.preventDefault();
    applied = filter.trim();
    starts = [0];
    void load();
  }

  function next() {
    if (page?.next == null) return;
    starts = [...starts, page.next];
    void load();
  }

  function back() {
    if (starts.length < 2) return;
    starts = starts.slice(0, -1);
    void load();
  }

  async function save(format: DbBackupFormat) {
    if (!token || selected === null || saving) return;
    saving = { format, rows: 0 };
    await listen();
    try {
      const r = await api.dbBackupExport(token, selected, format, applied || null, newTransfer());
      if (r) toasts.success(t("backup.saved", { n: r.rows.toLocaleString(), path: r.path }));
    } catch (e) {
      toasts.error(e);
    } finally {
      saving = null;
    }
  }

  function stop() {
    if (transferId) void api.dbTransferCancel(transferId);
  }

  onMount(() => void open());
  onDestroy(() => {
    unlisten?.();
    if (phase === "indexing" || saving) stop();
    if (token) void api.dbBackupClose(token);
    delete backups.names[leaf];
  });

  const shownFrom = $derived(page?.positions[0] ?? 0);
  const shownTo = $derived(page && page.positions.length ? page.positions[page.positions.length - 1] + 1 : 0);
</script>

<div class="backup">
  {#if phase === "picking" || phase === "indexing"}
    <div class="center">
      <Icon name="database" size={22} />
      <p>{phase === "picking" ? t("backup.picking") : t("backup.indexing", { bytes: formatBytes(readBytes) })}</p>
      {#if phase === "indexing"}<button class="btn sm" onclick={stop}>{t("db.xfer.stop")}</button>{/if}
    </div>
  {:else if phase === "failed"}
    <div class="center">
      <p class="error" role="alert">{error}</p>
      <div class="row">
        <button class="btn sm primary" onclick={open}>{t("backup.another")}</button>
        <button class="btn sm ghost" onclick={onclose}>{t("common.close")}</button>
      </div>
    </div>
  {:else if index && file}
    <header>
      <Icon name="database" size={14} />
      <strong class="mono name" title={file.name}>{file.name}</strong>
      <span class="faint">{formatBytes(file.size)}{file.compressed ? " · gzip" : ""} · {t("backup.summary", { tables: index.tables.length, statements: index.statements.toLocaleString() })}</span>
      <span class="spacer"></span>
      <label class="faint readas">
        {t("backup.readAs")}
        <select class="select" value={index.kind} onchange={(e) => reindex(e.currentTarget.value as "mysql" | "postgres")}>
          <option value="mysql">MySQL / MariaDB</option>
          <option value="postgres">PostgreSQL</option>
        </select>
      </label>
      <button class="btn sm ghost" onclick={open}>{t("backup.another")}</button>
    </header>
    <div class="body">
      <aside>
        <input class="input" bind:value={nameFilter} placeholder={t("backup.findTable")} aria-label={t("backup.findTable")} spellcheck="false" />
        <ul role="listbox" aria-label={t("backup.tables")}>
          {#each tables as { tb, i } (i)}
            <li>
              <button role="option" aria-selected={selected === i} class:on={selected === i} onclick={() => pick(i)}>
                <Icon name={tb.view ? "eye" : "layout"} size={12} />
                <span class="mono tname">{tb.schema && index.kind === "postgres" ? `${tb.schema}.` : ""}{tb.name}</span>
                {#if !tb.view}<span class="count">{tb.rows.toLocaleString()}</span>{/if}
              </button>
            </li>
          {/each}
          {#if !index.tables.length}<li class="faint pad">{t("backup.noTables")}</li>{/if}
        </ul>
      </aside>
      <section>
        {#if table && selected !== null}
          <div class="bar">
            <strong class="mono">{table.name}</strong>
            <span class="faint">{table.view ? t("db.xfer.view") : t("backup.rows", { n: table.rows.toLocaleString() })}</span>
            <div class="seg" role="tablist">
              <button role="tab" aria-selected={view === "data"} class:on={view === "data"} disabled={table.view} onclick={() => ((view = "data"), !page && load())}>{t("backup.data")}</button>
              <button role="tab" aria-selected={view === "structure"} class:on={view === "structure"} onclick={() => (view = "structure")}>{t("backup.structure")}</button>
            </div>
            <span class="spacer"></span>
            {#if saving}
              <span class="faint">{t("backup.saving", { n: saving.rows.toLocaleString() })}</span>
              <button class="btn sm" onclick={stop}>{t("db.xfer.stop")}</button>
            {:else}
              <span class="saveas">
                <span class="faint">{applied ? t("backup.saveMatching") : t("backup.saveAs")}</span>
                {#if !table.view}
                  <button class="btn sm" onclick={() => save("csv")}>CSV</button>
                  <button class="btn sm" onclick={() => save("json")}>JSON</button>
                {/if}
                <button class="btn sm" onclick={() => save("sql")} title={t("backup.sqlHint")}>SQL</button>
              </span>
            {/if}
          </div>
          {#if view === "data"}
            <form class="bar" onsubmit={applyFilter}>
              <input class="input filter" bind:value={filter} placeholder={t("backup.filter")} aria-label={t("backup.filter")} spellcheck="false" />
              <button class="btn sm" type="submit">{t("backup.apply")}</button>
              {#if applied}<button class="btn sm ghost" type="button" onclick={() => ((filter = ""), applyFilter(new Event("submit")))}>{t("backup.clear")}</button>{/if}
              <span class="spacer"></span>
              {#if page}
                <span class="faint">
                  {#if !page.rows.length}
                    {t("backup.none")}
                  {:else if applied}
                    {t("backup.matching", { n: page.rows.length, from: (shownFrom + 1).toLocaleString(), to: shownTo.toLocaleString() })}
                  {:else}
                    {t("backup.range", { from: (shownFrom + 1).toLocaleString(), to: shownTo.toLocaleString(), total: table.rows.toLocaleString() })}
                  {/if}
                </span>
              {/if}
              <button class="icon-btn" type="button" aria-label={t("backup.prev")} title={t("backup.prev")} disabled={starts.length < 2 || loading} onclick={back}><Icon name="chevronLeft" size={12} /></button>
              <button class="icon-btn" type="button" aria-label={t("backup.next")} title={t("backup.next")} disabled={page?.next == null || loading} onclick={next}><Icon name="chevronRight" size={12} /></button>
            </form>
            {#if pageError}
              <p class="error" role="alert">{pageError}</p>
            {:else if result}
              <div class="grid"><DbGrid {result} firstRow={applied ? 0 : shownFrom} /></div>
            {:else}
              <p class="faint pad">{t("db.loading")}</p>
            {/if}
          {:else}
            <div class="structure">
              {#if table.columns.length}
                <p class="faint">{t("backup.columns")}: <span class="mono">{table.columns.join(", ")}</span></p>
              {/if}
              <pre class="mono">{table.create ?? t("backup.noCreate")}</pre>
            </div>
          {/if}
        {:else}
          <p class="faint pad">{t("backup.pick")}</p>
        {/if}
      </section>
    </div>
  {/if}
</div>

<style>
  .backup {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    font-size: 12px;
  }
  .center {
    margin: auto;
    display: grid;
    justify-items: center;
    gap: 10px;
    color: var(--muted);
    text-align: center;
    max-width: 480px;
    padding: 20px;
  }
  .row {
    display: flex;
    gap: 8px;
  }
  header,
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    flex-wrap: wrap;
  }
  header .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 320px;
  }
  .readas,
  .saveas {
    display: flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
  }
  header .select {
    height: 24px;
    font-size: 12px;
    margin-left: 4px;
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  aside {
    width: 240px;
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  aside .input {
    margin: 8px;
    width: calc(100% - 16px);
    height: 26px;
    font-size: 12px;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0 4px 8px;
    overflow: auto;
    flex: 1;
  }
  li button {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 6px;
    border: 0;
    background: none;
    color: var(--text);
    font: inherit;
    padding: 4px 6px;
    border-radius: 5px;
    cursor: pointer;
    text-align: left;
  }
  li button:hover {
    background: var(--hover, var(--accent-soft));
  }
  li button.on {
    background: var(--accent-soft);
  }
  .tname {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .count {
    color: var(--faint);
    font-variant-numeric: tabular-nums;
  }
  section {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .seg {
    display: flex;
    border: 1px solid var(--border);
    border-radius: 6px;
    overflow: hidden;
  }
  .seg button {
    border: 0;
    background: none;
    color: var(--muted);
    font: inherit;
    padding: 3px 8px;
    cursor: pointer;
  }
  .seg button.on {
    background: var(--accent-soft);
    color: var(--text);
  }
  .filter {
    width: 260px;
    height: 26px;
    font-size: 12px;
  }
  .grid {
    flex: 1;
    min-height: 0;
  }
  .structure {
    overflow: auto;
    padding: 10px;
    flex: 1;
  }
  pre {
    margin: 8px 0 0;
    white-space: pre-wrap;
    word-break: break-word;
    font-size: 12px;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .faint {
    color: var(--faint);
  }
  .pad {
    padding: 10px;
  }
  .spacer {
    flex: 1;
  }
  .error {
    color: var(--danger);
    font-family: var(--font-mono);
    padding: 8px 10px;
    margin: 0;
  }
</style>
