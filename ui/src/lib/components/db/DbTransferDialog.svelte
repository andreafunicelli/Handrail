<script lang="ts">
  import { onDestroy } from "svelte";
  import Modal from "../common/Modal.svelte";
  import { api } from "../../api";
  import type { DbColumnInfo, DbPickedFile, DbTableInfo, DbTransferProgress, DbTransferSummary } from "../../api/types";
  import { errorMessage } from "../../api/errors";
  import { t } from "../../i18n";
  import { toasts } from "../../stores/toasts.svelte";
  import { formatBytes } from "../../util/time";

  /** Export a namespace to an SQL file, or import an SQL or CSV file. The backend reads and
   *  writes the file on a connection of its own; this shows how far it got and can stop it. */
  let {
    id,
    dialect,
    prod,
    mode,
    namespace,
    namespaces,
    tables,
    table = null,
    ondone,
    onclose,
  }: {
    id: string;
    dialect: "mysql" | "postgres";
    prod: boolean;
    mode: "export" | "sql" | "csv";
    namespace: string;
    namespaces: string[];
    /** The namespace's tables and views (export: the ones to pick from). */
    tables: DbTableInfo[];
    /** Export: only this one checked; CSV: the table filled. */
    table?: string | null;
    /** Something was imported: the schema tree reloads. */
    ondone: () => void;
    onclose: () => void;
  } = $props();

  let phase = $state<"setup" | "running" | "done">("setup");
  let error = $state<string | null>(null);
  let progress = $state<DbTransferProgress | null>(null);
  let summary = $state<DbTransferSummary | null>(null);
  let savedTo = $state<string | null>(null);
  let transferId = "";
  let unlisten: (() => void) | null = null;

  // Export.
  // svelte-ignore state_referenced_locally
  let picked = $state<string[]>(table ? [table] : tables.map((x) => x.name));
  let structure = $state(true);
  let data = $state(true);
  let drop = $state(false);

  // Import.
  let file = $state<DbPickedFile | null>(null);
  // svelte-ignore state_referenced_locally
  let target = $state(namespace);
  let stopOnError = $state(true);
  let typed = $state("");

  // CSV.
  let delimiter = $state(";");
  let header = $state(true);
  let nullText = $state<string>("\\N");
  let replace = $state(false);
  let preview = $state<string[][]>([]);
  let mapping = $state<(string | null)[]>([]);
  let tableColumns = $state<DbColumnInfo[]>([]);

  const DELIMITERS: [string, string][] = [
    [";", ";"],
    [",", ","],
    ["\t", "Tab"],
    ["|", "|"],
  ];
  const NULLS: [string, string][] = [
    ["\\N", "\\N"],
    ["", t("db.xfer.nullEmpty")],
    ["NULL", "NULL"],
    ["(none)", t("db.xfer.nullNone")],
  ];

  const title = $derived(
    mode === "export" ? t("db.xfer.exportTitle", { name: namespace }) : mode === "sql" ? t("db.xfer.sqlTitle") : t("db.xfer.csvTitle", { name: `${namespace}.${table ?? ""}` }),
  );
  /** Typing the name: an import into production, or one that deletes rows first. */
  const confirmName = $derived(mode === "export" ? null : prod || (mode === "csv" && replace) ? (mode === "csv" ? (table ?? "") : target) : null);
  const ready = $derived.by(() => {
    if (phase !== "setup") return false;
    if (confirmName !== null && typed.trim() !== confirmName) return false;
    if (mode === "export") return picked.length > 0 && (structure || data);
    if (!file) return false;
    if (mode === "csv") return mapping.some((m) => m);
    return true;
  });

  async function pick() {
    error = null;
    try {
      const f = await api.dbPickFile(mode === "csv" ? "csv" : "sql");
      if (!f) return;
      file = f;
      if (mode === "csv") await loadPreview(true);
    } catch (e) {
      error = errorMessage(e);
    }
  }

  async function loadPreview(remap: boolean) {
    if (!file || mode !== "csv") return;
    try {
      if (!tableColumns.length && table) tableColumns = await api.dbColumns(id, namespace, table);
      preview = await api.dbCsvPreview(file.token, delimiter);
      if (remap || mapping.length !== (preview[0]?.length ?? 0)) automap();
    } catch (e) {
      error = errorMessage(e);
      preview = [];
    }
  }

  /** Fields to columns: by header name, else by position. */
  function automap() {
    const first = preview[0] ?? [];
    const names = tableColumns.map((c) => c.name);
    mapping = first.map((h, i) => {
      if (header) return names.find((n) => n.toLowerCase() === h.trim().toLowerCase()) ?? null;
      return names[i] ?? null;
    });
  }

  function toggle(name: string) {
    picked = picked.includes(name) ? picked.filter((x) => x !== name) : [...picked, name];
  }

  async function start(e: Event) {
    e.preventDefault();
    if (!ready) return;
    error = null;
    transferId = `dbx-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
    progress = null;
    unlisten?.();
    unlisten = await api.on("db://transfer", (ev) => {
      if (ev.transfer_id === transferId) progress = ev.progress;
    });
    phase = "running";
    try {
      if (mode === "export") {
        const all = picked.length === tables.length;
        const r = await api.dbDump(id, namespace, { tables: all ? [] : $state.snapshot(picked), structure, data, drop: structure && drop }, transferId);
        if (!r) {
          phase = "setup";
          return;
        }
        summary = r.summary;
        savedTo = r.path || null;
      } else if (mode === "sql") {
        summary = await api.dbImportSql(id, file!.token, target || null, stopOnError, transferId);
        ondone();
      } else {
        summary = await api.dbImportCsv(
          id,
          file!.token,
          namespace,
          table ?? "",
          { delimiter, header, null: nullText === "(none)" ? null : nullText, columns: $state.snapshot(mapping), replace },
          transferId,
        );
        ondone();
      }
      phase = "done";
      if (!summary.cancelled && !summary.errors.length) toasts.success(t(mode === "export" ? "db.xfer.exported" : "db.xfer.imported"));
    } catch (err) {
      error = errorMessage(err);
      phase = "setup";
    } finally {
      unlisten?.();
      unlisten = null;
    }
  }

  function cancel() {
    if (phase === "running") void api.dbTransferCancel(transferId);
    else onclose();
  }

  onDestroy(() => {
    unlisten?.();
    if (phase === "running") void api.dbTransferCancel(transferId);
  });

  const shown = $derived(summary?.progress ?? progress);
  const pct = $derived.by(() => {
    if (!shown) return null;
    if (mode === "export") return shown.tables ? Math.round((100 * shown.tables_done) / shown.tables) : null;
    return file?.size ? Math.min(100, Math.round((100 * shown.bytes) / file.size)) : null;
  });
</script>

<Modal label={title} onclose={cancel} dismissable={phase !== "running"} width="600px">
  <form class="pad" onsubmit={start} autocomplete="off">
    <h2>{title}</h2>

    {#if phase === "setup"}
      {#if mode === "export"}
        <div class="row">
          <label class="check"><input type="checkbox" bind:checked={structure} />{t("db.xfer.structure")}</label>
          <label class="check"><input type="checkbox" bind:checked={data} />{t("db.xfer.data")}</label>
          <label class="check" class:off={!structure}><input type="checkbox" bind:checked={drop} disabled={!structure} />{t("db.xfer.drop")}</label>
        </div>
        <div class="listhead">
          <span class="label">{t("db.xfer.tables", { n: picked.length, total: tables.length })}</span>
          <button type="button" class="link" onclick={() => (picked = tables.map((x) => x.name))}>{t("db.xfer.all")}</button>
          <button type="button" class="link" onclick={() => (picked = [])}>{t("db.xfer.none")}</button>
        </div>
        <div class="list">
          {#each tables as tb (tb.name)}
            <label class="check"><input type="checkbox" checked={picked.includes(tb.name)} onchange={() => toggle(tb.name)} /><span class="mono">{tb.name}</span>{#if tb.view}<span class="faint">{t("db.xfer.view")}</span>{/if}</label>
          {/each}
        </div>
        <p class="note">{t(dialect === "mysql" ? "db.xfer.exportNoteMysql" : "db.xfer.exportNotePg")}</p>
      {:else}
        <div class="row">
          <button type="button" class="btn" onclick={pick}>{file ? t("db.xfer.another") : t("db.xfer.pick")}</button>
          {#if file}<span class="mono file">{file.name}</span><span class="faint">{formatBytes(file.size)}</span>{/if}
        </div>
        {#if mode === "sql"}
          <label class="field">
            <span class="label">{t(dialect === "mysql" ? "db.xfer.intoDatabase" : "db.xfer.intoSchema")}</span>
            <select class="select" bind:value={target}>
              {#each namespaces as ns (ns)}<option value={ns}>{ns}</option>{/each}
            </select>
          </label>
          <label class="check"><input type="checkbox" bind:checked={stopOnError} />{t("db.xfer.stopOnError")}</label>
          <p class="note">{t(dialect === "postgres" ? (stopOnError ? "db.xfer.sqlNotePgAtomic" : "db.xfer.sqlNotePg") : "db.xfer.sqlNoteMysql")}</p>
        {:else if file}
          <div class="row">
            <label class="field">
              <span class="label">{t("db.xfer.delimiter")}</span>
              <select class="select" bind:value={delimiter} onchange={() => loadPreview(true)}>
                {#each DELIMITERS as [v, l] (v)}<option value={v}>{l}</option>{/each}
              </select>
            </label>
            <label class="field">
              <span class="label">{t("db.xfer.nullAs")}</span>
              <select class="select" bind:value={nullText}>
                {#each NULLS as [v, l] (v)}<option value={v}>{l}</option>{/each}
              </select>
            </label>
            <label class="check"><input type="checkbox" bind:checked={header} onchange={automap} />{t("db.xfer.header")}</label>
          </div>
          {#if preview.length}
            <div class="preview">
              <table>
                <thead>
                  <tr>
                    {#each preview[0] as _, i (i)}
                      <th>
                        <select class="select" bind:value={mapping[i]} aria-label={t("db.xfer.columnFor", { n: i + 1 })}>
                          <option value={null}>{t("db.xfer.skip")}</option>
                          {#each tableColumns as c (c.name)}<option value={c.name}>{c.name}</option>{/each}
                        </select>
                      </th>
                    {/each}
                  </tr>
                </thead>
                <tbody>
                  {#each preview as r, ri (ri)}
                    <tr class:head={header && ri === 0}>
                      {#each r as v, ci (ci)}<td class="mono" class:null={!(header && ri === 0) && nullText !== "(none)" && v === nullText}>{v}</td>{/each}
                    </tr>
                  {/each}
                </tbody>
              </table>
            </div>
          {/if}
          <label class="check danger"><input type="checkbox" bind:checked={replace} />{t("db.xfer.replace")}</label>
          <p class="note">{t("db.xfer.csvNote")}</p>
        {/if}
        {#if confirmName !== null && (mode === "sql" || file)}
          <label class="field">
            <span class="label">{t("db.ddl.typeToConfirm", { name: confirmName })}</span>
            <input class="input mono" bind:value={typed} placeholder={confirmName} spellcheck="false" />
          </label>
        {/if}
      {/if}
    {/if}

    {#if phase !== "setup" && shown}
      <div class="progress" aria-live="polite">
        {#if pct !== null}<div class="bar"><div class="fill" style:width="{pct}%"></div></div>{/if}
        <div class="stats">
          {#if shown.table && phase === "running"}<span class="mono">{shown.table}</span>{/if}
          {#if mode === "export"}<span>{t("db.xfer.tablesDone", { n: shown.tables_done, total: shown.tables })}</span>{/if}
          <span>{t("db.xfer.rows", { n: shown.rows.toLocaleString() })}</span>
          {#if mode !== "csv"}<span>{t("db.xfer.statements", { n: shown.statements.toLocaleString() })}</span>{/if}
          <span>{formatBytes(shown.bytes)}{mode !== "export" && file ? ` / ${formatBytes(file.size)}` : ""}</span>
        </div>
      </div>
    {:else if phase === "running"}
      <p class="note">{t("db.running")}</p>
    {/if}

    {#if phase === "done" && summary}
      {#if summary.cancelled}
        <p class="warn">{t(summary.rolled_back ? "db.xfer.cancelledRolledBack" : mode === "export" ? "db.xfer.cancelledExport" : "db.xfer.cancelledKept")}</p>
      {:else if summary.errors.length}
        <p class="warn">{t(summary.rolled_back ? "db.xfer.failedRolledBack" : "db.xfer.failed", { n: summary.progress.errors })}</p>
      {:else}
        <p class="ok">{mode === "export" && savedTo ? t("db.xfer.savedTo", { path: savedTo }) : t("db.xfer.imported")}</p>
      {/if}
      {#if summary.errors.length}
        <ul class="errors">
          {#each summary.errors as er, i (i)}
            <li>
              {#if er.line}<span class="faint">{t("db.xfer.line", { n: er.line })}</span>{/if}
              <span class="msg">{er.message}</span>
              <code>{er.statement}</code>
            </li>
          {/each}
        </ul>
      {/if}
    {/if}

    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <div class="actions">
      {#if phase === "running"}
        <button type="button" class="btn danger" onclick={cancel}>{t("db.xfer.stop")}</button>
      {:else}
        <button type="button" class="btn ghost" onclick={onclose}>{t("common.close")}</button>
        {#if phase === "setup"}
          <button type="submit" class="btn primary" disabled={!ready}>{mode === "export" ? t("db.xfer.export") : t("db.xfer.import")}</button>
        {/if}
      {/if}
    </div>
  </form>
</Modal>

<style>
  .pad {
    padding: 20px;
    display: grid;
    gap: 10px;
  }
  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }
  p {
    margin: 0;
  }
  .row {
    display: flex;
    gap: 10px;
    align-items: flex-end;
    flex-wrap: wrap;
  }
  .field {
    display: grid;
    gap: 4px;
    flex: 1;
  }
  .label {
    font-size: 12px;
    color: var(--muted);
  }
  .check {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
  }
  .check.off {
    opacity: 0.5;
  }
  .check.danger {
    color: var(--danger);
  }
  .listhead {
    display: flex;
    gap: 10px;
    align-items: center;
  }
  .link {
    border: 0;
    background: none;
    color: var(--accent);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
    padding: 0;
  }
  .list {
    max-height: 200px;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 6px 8px;
    display: grid;
    gap: 3px;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .faint {
    color: var(--faint);
    font-size: 12px;
  }
  .file {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 320px;
  }
  .note {
    color: var(--muted);
    font-size: 12px;
  }
  .preview {
    overflow: auto;
    max-height: 220px;
    border: 1px solid var(--border);
    border-radius: 6px;
  }
  table {
    border-collapse: collapse;
    font-size: 12px;
    width: 100%;
  }
  th {
    padding: 4px;
    background: var(--bg-soft, var(--bg));
    position: sticky;
    top: 0;
  }
  th .select {
    height: 24px;
    font-size: 12px;
    min-width: 100px;
  }
  td {
    padding: 3px 6px;
    border-top: 1px solid var(--border);
    white-space: nowrap;
    max-width: 220px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  tr.head td {
    color: var(--faint);
    text-decoration: line-through;
  }
  td.null {
    color: var(--faint);
    font-style: italic;
  }
  .progress {
    display: grid;
    gap: 6px;
  }
  .bar {
    height: 6px;
    border-radius: 3px;
    background: var(--border);
    overflow: hidden;
  }
  .fill {
    height: 100%;
    background: var(--accent);
    transition: width 0.2s;
  }
  .stats {
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
    font-size: 12px;
    color: var(--muted);
  }
  .ok {
    color: var(--ok, var(--text));
    word-break: break-all;
  }
  .warn {
    color: var(--warn);
  }
  .error {
    color: var(--danger);
    font-family: var(--font-mono);
    font-size: 12px;
  }
  .errors {
    margin: 0;
    padding: 0;
    list-style: none;
    max-height: 180px;
    overflow: auto;
    display: grid;
    gap: 6px;
    font-size: 12px;
  }
  .errors li {
    display: grid;
    gap: 2px;
    border-left: 2px solid var(--danger);
    padding-left: 8px;
  }
  .errors code {
    color: var(--faint);
    white-space: pre-wrap;
    word-break: break-all;
    font-size: 11px;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 4px;
  }
</style>
