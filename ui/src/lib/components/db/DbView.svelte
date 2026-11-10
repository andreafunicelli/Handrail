<script lang="ts">
  import { onMount, untrack } from "svelte";
  import Icon from "../icons/Icon.svelte";
  import ContextMenu, { type MenuEntry } from "../common/ContextMenu.svelte";
  import DbGrid from "./DbGrid.svelte";
  import { api } from "../../api";
  import type { DbColumnInfo, DbOpened, DbQueryOutput, DbTableInfo } from "../../api/types";
  import { errorMessage, isIpcError } from "../../api/errors";
  import { t } from "../../i18n";
  import { connections } from "../../stores/connections.svelte";
  import { dbApprovals } from "../../stores/dbApprovals.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { copyText } from "../../util/clipboard";
  import { createSqlEditor, type SqlEditorHandle } from "../../util/codemirror";
  import { buildEdits, editCount, noEdits, primaryResult, resultSummary, tableOverview, tableStats, toCsv, toInserts, toJson, type GridEdits } from "../../util/db";
  import { addHistory, clearHistory, loadHistory, loadTabs, nextTabName, saveTabs, type HistoryEntry, type QueryTabDoc } from "../../util/dbtabs";
  import { statementAt } from "../../util/sqltext";
  import { relativeTime } from "../../util/time";

  /** A database in a pane: schema on the left, SQL editor over the results on the right. */
  let { connectionId, focused }: { connectionId: string; focused: boolean } = $props();

  const conn = $derived(connections.byId.get(connectionId));
  const ROW_CHOICES = [100, 1000, 10000];

  let phase = $state<"connecting" | "password" | "live" | "failed">("connecting");
  let problem = $state<string | null>(null);
  let password = $state("");
  let info = $state<DbOpened | null>(null);
  let id: string | null = null;
  /** `id`, for what has to follow it on screen. */
  let tabId = $state<string | null>(null);
  let closed = false;
  /** Shared with agents (off for every new connection). */
  let shared = $state(false);
  let denyNote = $state<Record<string, string>>({});
  const asks = $derived(dbApprovals.forTab(tabId));

  let namespaces = $state<string[]>([]);
  /** Tables per namespace, loaded when the namespace is opened. */
  let tables = $state<Record<string, DbTableInfo[] | "loading">>({});
  let openNs = $state<Record<string, boolean>>({});
  /** Columns per `namespace.table`, loaded when the table is opened in the tree. */
  let columns = $state<Record<string, DbColumnInfo[] | "loading">>({});
  let openTables = $state<Record<string, boolean>>({});

  let editorHost = $state<HTMLDivElement>();
  let editor: SqlEditorHandle | null = null;
  let running = $state(false);
  let output = $state<DbQueryOutput | null>(null);
  let active = $state(0);
  let maxRows = $state(1000);
  let ranLabel = $state("");

  /** The table whose rows are shown, and which page of them. */
  let browse = $state<{
    ns: string;
    table: string;
    offset: number;
    /** Offsets of the pages before this one (Previous goes back to the last). */
    back: number[];
    /** The row limit this page was loaded with. */
    limit: number;
    orderBy: string | null;
    desc: boolean;
    filter: string;
  } | null>(null);
  let filterText = $state("");
  /** A table's CREATE statement, shown over the results. */
  let ddl = $state<{ title: string; text: string } | null>(null);
  const tableInfo = (ns: string, table: string) => (Array.isArray(tables[ns]) ? (tables[ns] as DbTableInfo[]).find((x) => x.name === table) : undefined);

  /** The table whose rows are shown, when they can be edited (it has a primary key). */
  let editTarget = $state<{ ns: string; table: string; pk: string[] } | null>(null);
  let edits = $state<GridEdits | null>(null);
  let selected = $state<{ r: number; c: number } | null>(null);
  let grid = $state<{ setCell(r: number, c: number, v: string | null): void; reveal(r: number): void }>();
  let saving = $state(false);
  const pending = $derived(edits ? editCount(edits) : 0);

  /** Unsaved edits block anything that would replace the rows. */
  function blocked(): boolean {
    if (!pending) return false;
    toasts.info(t("db.unsaved"));
    return true;
  }

  // ----- query tabs: each keeps its SQL (saved between sessions) and, while open, its results.
  const store = (() => {
    try {
      return localStorage;
    } catch {
      return null;
    }
  })();
  // A view belongs to one connection for its whole life.
  const connKey = untrack(() => connectionId);
  const initial = store ? loadTabs(store, connKey, () => crypto.randomUUID()) : { tabs: [{ key: "q1", name: "Query 1", sql: "" }], current: 0 };
  let qtabs = $state<QueryTabDoc[]>(initial.tabs);
  let qcur = $state(initial.current);
  type View = {
    output: DbQueryOutput | null;
    active: number;
    ranLabel: string;
    browse: typeof browse;
    filterText: string;
    editTarget: typeof editTarget;
  };
  /** What each other tab showed, by tab key. */
  const views: Record<string, View> = {};
  const persistTabs = () => store && saveTabs(store, connKey, $state.snapshot(qtabs) as QueryTabDoc[], qcur);

  function switchTab(i: number) {
    if (i === qcur || running || blocked()) return;
    views[qtabs[qcur].key] = { output, active, ranLabel, browse, filterText, editTarget };
    qcur = i;
    const v = views[qtabs[i].key];
    output = v?.output ?? null;
    active = v?.active ?? 0;
    ranLabel = v?.ranLabel ?? "";
    browse = v?.browse ?? null;
    filterText = v?.filterText ?? "";
    editTarget = v?.editTarget ?? null;
    edits = editTarget ? noEdits() : null;
    ddl = null;
    selected = null;
    editor?.setText(qtabs[i].sql);
    persistTabs();
    editor?.focus();
  }

  function newTab() {
    if (running || blocked()) return;
    qtabs.push({ key: crypto.randomUUID(), name: nextTabName(qtabs), sql: "" });
    switchTab(qtabs.length - 1);
  }

  function closeTab(i: number) {
    if (qtabs.length < 2 || running) return;
    if (i === qcur) {
      if (blocked()) return;
      switchTab(i > 0 ? i - 1 : 1);
    }
    delete views[qtabs[i].key];
    qtabs.splice(i, 1);
    if (qcur > i) qcur--;
    persistTabs();
  }

  // ----- history of what was run.
  let history = $state<HistoryEntry[]>(store ? loadHistory(store, connKey) : []);
  let menu = $state<{ x: number; y: number; items: MenuEntry[] } | null>(null);
  const oneLine = (sql: string) => {
    const s = sql.replace(/\s+/g, " ").trim();
    return s.length > 70 ? `${s.slice(0, 70)}…` : s;
  };

  function openHistory(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const items: MenuEntry[] = history.slice(0, 30).map((h) => ({
      label: `${h.ok ? "" : "✕ "}${oneLine(h.sql)}`,
      shortcut: relativeTime(h.at),
      action: () => editor?.append(h.sql),
    }));
    if (!items.length) items.push({ label: t("db.historyEmpty"), disabled: true, action: () => {} });
    else
      items.push("sep", {
        label: t("db.historyClear"),
        icon: "trash",
        danger: true,
        action: () => {
          if (store) clearHistory(store, connKey);
          history = [];
        },
      });
    menu = { x: r.left, y: r.bottom + 4, items };
  }

  async function connect(pw?: string) {
    phase = "connecting";
    problem = null;
    try {
      const opened = await api.dbOpen(connectionId, pw);
      if (closed) return void api.dbClose(opened.id).catch(() => {});
      id = opened.id;
      tabId = opened.id;
      shared = false;
      info = opened;
      phase = "live";
      password = "";
      await loadNamespaces();
    } catch (e) {
      if (isIpcError(e) && e.kind === "auth" && e.message === "password_required") {
        phase = "password";
        return;
      }
      phase = "failed";
      problem = errorMessage(e);
    }
  }

  async function loadNamespaces() {
    if (!id) return;
    try {
      namespaces = await api.dbNamespaces(id);
      const home = defaultNamespace();
      if (home && namespaces.includes(home)) await toggleNs(home, true);
    } catch (e) {
      toasts.error(e);
    }
  }

  /** The namespace queries run in by default: the connection's database (MySQL) or `public`. */
  function defaultNamespace(): string | null {
    if (info?.kind === "postgres") return "public";
    return info?.database ?? conn?.database ?? null;
  }

  async function toggleNs(ns: string, open = !openNs[ns]) {
    openNs[ns] = open;
    if (!open || !id || (tables[ns] && tables[ns] !== "loading")) return;
    tables[ns] = "loading";
    try {
      tables[ns] = await api.dbTables(id, ns);
      updateCompletion();
    } catch (e) {
      delete tables[ns];
      toasts.error(e);
    }
  }

  async function toggleTable(ns: string, table: string) {
    const key = `${ns}.${table}`;
    openTables[key] = !openTables[key];
    if (!openTables[key] || !id || columns[key]) return;
    columns[key] = "loading";
    try {
      columns[key] = await api.dbColumns(id, ns, table);
      updateCompletion();
    } catch (e) {
      delete columns[key];
      toasts.error(e);
    }
  }

  /** Completion knows every loaded table, with its columns once they were loaded. */
  function updateCompletion() {
    const schema: Record<string, string[]> = {};
    for (const [ns, list] of Object.entries(tables)) {
      if (list === "loading") continue;
      for (const tb of list) {
        const cols = columns[`${ns}.${tb.name}`];
        schema[`${ns}.${tb.name}`] = Array.isArray(cols) ? cols.map((c) => c.name) : [];
      }
    }
    editor?.setSchema(schema, defaultNamespace() ?? undefined);
  }

  /** Run the selection, or everything. */
  async function run() {
    if (!editor) return;
    await runSql(editor.runText().trim());
  }

  /** Run the statement the cursor is in (selected while it runs). */
  async function runCurrent() {
    if (!editor || running) return;
    const st = statementAt(editor.text(), editor.cursor(), info?.kind ?? "mysql");
    if (!st) return;
    editor.select(st.from, st.to);
    await runSql(st.text);
  }

  async function runSql(sql: string) {
    if (!id || running || !sql || blocked()) return;
    editTarget = null;
    edits = null;
    browse = null;
    ddl = null;
    ranLabel = sql.length > 80 ? `${sql.slice(0, 80)}…` : sql;
    await execute(() => api.dbQuery(id!, sql, maxRows));
    if (store && !closed) history = addHistory(store, connKey, { sql, at: Date.now(), ms: output?.elapsed_ms ?? 0, ok: !output?.error });
  }

  /** Show a table's rows from the first page; `keep` stays on the page, sort and filter shown. */
  async function openTable(ns: string, table: string, force = false, keep = false) {
    if (!id || running || (!force && blocked())) return;
    if (!keep || browse?.ns !== ns || browse.table !== table) {
      browse = { ns, table, offset: 0, back: [], limit: maxRows, orderBy: null, desc: false, filter: "" };
      filterText = "";
    }
    const b = browse!;
    b.limit = maxRows;
    ranLabel = `${ns}.${table}`;
    editTarget = null;
    edits = null;
    ddl = null;
    const seq = await execute(() => api.dbTableRows(id!, ns, table, { maxRows, offset: b.offset, orderBy: b.orderBy, desc: b.desc, filter: b.filter }));
    const r = output?.results[0];
    const view = tableInfo(ns, table)?.view;
    if (!id || closed || !r || output?.error || output?.results.length !== 1 || view) return;
    try {
      const key = `${ns}.${table}`;
      const cols = Array.isArray(columns[key]) ? (columns[key] as DbColumnInfo[]) : await api.dbColumns(id, ns, table);
      columns[key] = cols;
      // Something else ran while the columns loaded: these rows are no longer the ones shown.
      if (seq !== runs || closed) return;
      // A query without rows reports no columns: show the table's own, so rows can be added.
      if (!r.rows.length && !r.columns.length) r.columns = cols.map((c) => ({ name: c.name, type_name: c.data_type }));
      const pk = cols.filter((c) => c.primary_key).map((c) => c.name);
      const names = r.columns.map((c) => c.name);
      // Rows are found by their primary key: without one (or without all of its columns) the
      // grid stays read-only.
      if (pk.length && pk.every((k) => names.includes(k))) {
        editTarget = { ns, table, pk };
        edits = noEdits();
      }
    } catch {
      // Columns could not be read: the rows stay read-only.
    }
  }

  /** Change the page, sort or filter of the table shown, and load it again. */
  function reshow(change: (b: NonNullable<typeof browse>) => void) {
    if (!browse || running || blocked()) return;
    change(browse);
    void openTable(browse.ns, browse.table, true, true);
  }
  /** Next starts after the rows shown (fewer than the limit when the size cap cut the page);
   *  Previous goes back to where the page before started. */
  const page = (dir: 1 | -1) =>
    reshow((b) => {
      if (dir > 0) {
        b.back.push(b.offset);
        b.offset += output?.results[0]?.rows.length ?? 0;
      } else b.offset = b.back.pop() ?? 0;
    });
  /** Whether rows may follow the page shown. */
  const hasMore = $derived.by(() => {
    const r = output?.results[0];
    return !!browse && !!r && r.rows.length > 0 && (r.truncated || r.rows.length >= browse.limit);
  });
  /** A header click sorts ascending, then descending, then back to the table's order. */
  const sortBy = (col: string) =>
    reshow((b) => {
      if (b.orderBy !== col) [b.orderBy, b.desc] = [col, false];
      else if (!b.desc) b.desc = true;
      else [b.orderBy, b.desc] = [null, false];
      b.offset = 0;
      b.back = [];
    });
  const applyFilter = () =>
    reshow((b) => {
      b.filter = filterText.trim();
      b.offset = 0;
      b.back = [];
    });

  async function showDdl(ns: string, table: string) {
    if (!id) return;
    try {
      ddl = { title: `${ns}.${table}`, text: await api.dbTableDdl(id, ns, table) };
    } catch (e) {
      toasts.error(e);
    }
  }

  /** A namespace's tables with their size and estimated rows, in the grid. */
  async function overview(ns: string) {
    if (!id || running || blocked()) return;
    await toggleNs(ns, true);
    const list = tables[ns];
    if (!Array.isArray(list)) return;
    browse = null;
    editTarget = null;
    edits = null;
    ddl = null;
    ranLabel = ns;
    output = { results: [tableOverview(list)], error: null, elapsed_ms: 0 };
    active = 0;
  }

  function addRow() {
    if (!edits || saving || !output?.results[0]) return;
    edits.added.push({});
    const r = output.results[0].rows.length + edits.added.length - 1;
    selected = { r, c: 0 };
    queueMicrotask(() => grid?.reveal(r));
  }

  /** Mark the selected row for deletion (or drop it, if it is a new one); again to unmark. */
  function deleteRow() {
    if (!edits || saving || !selected || !output?.results[0]) return;
    const n = output.results[0].rows.length;
    if (selected.r >= n) {
      edits.added.splice(selected.r - n, 1);
      selected = null;
    } else if (edits.deleted[selected.r]) delete edits.deleted[selected.r];
    else edits.deleted[selected.r] = true;
  }

  function setNull() {
    if (selected) grid?.setCell(selected.r, selected.c, null);
  }

  async function save() {
    const target = editTarget;
    const r = output?.results[0];
    if (!id || !edits || !target || !r || !pending || saving) return;
    let list;
    try {
      list = buildEdits(r, target.pk, edits);
    } catch (e) {
      return void toasts.error(e);
    }
    saving = true;
    try {
      const n = await api.dbApplyEdits(id, target.ns, target.table, list);
      if (closed) return;
      toasts.success(t("db.saved", { n }));
      edits = noEdits();
      await openTable(target.ns, target.table, true, true);
    } catch (e) {
      toasts.error(e);
    } finally {
      saving = false;
    }
  }

  /** Counts what was run, so a late answer can tell it is about rows no longer shown. */
  let runs = 0;

  async function execute(call: () => Promise<DbQueryOutput>): Promise<number> {
    const seq = ++runs;
    running = true;
    try {
      const out = await call();
      if (closed) return seq;
      output = out;
      active = Math.max(0, primaryResult(out));
    } catch (e) {
      output = { results: [], error: errorMessage(e), elapsed_ms: 0 };
    } finally {
      running = false;
    }
    return seq;
  }

  async function toggleShare() {
    if (!id) return;
    try {
      await api.dbShare(id, !shared);
      shared = !shared;
      toasts.info(t(shared ? "db.sharedNow" : "db.unsharedNow"));
    } catch (e) {
      toasts.error(e);
    }
  }

  function decide(approvalId: string, approve: boolean) {
    const note = denyNote[approvalId]?.trim();
    delete denyNote[approvalId];
    void dbApprovals.decide(approvalId, approve ? { type: "approve" } : { type: "deny", message: note || null });
  }

  async function stop() {
    if (id) await api.dbCancel(id).catch((e) => toasts.error(e));
  }

  async function copyCsv() {
    const r = output?.results[active];
    if (!r) return;
    if (await copyText(toCsv(r))) toasts.success(t("db.csvCopied", { rows: r.rows.length }));
  }

  /** Save the rows shown to a file (CSV, JSON or INSERT statements into the table shown). */
  async function exportAs(format: "csv" | "json" | "sql") {
    const r = output?.results[active];
    if (!r) return;
    const base = (browse ? browse.table : "result").replace(/[^\w.-]+/g, "_");
    const path = browse ? [browse.ns, browse.table] : ["table_name"];
    const contents = format === "csv" ? toCsv(r) : format === "json" ? toJson(r) : toInserts(r, path, info?.kind ?? "mysql");
    try {
      const saved = await api.dbExportFile(`${base}.${format}`, contents);
      if (saved) toasts.success(t("db.exported", { rows: r.rows.length, path: saved }));
    } catch (e) {
      toasts.error(e);
    }
  }

  function openExport(e: MouseEvent) {
    const b = (e.currentTarget as HTMLElement).getBoundingClientRect();
    menu = {
      x: b.left,
      y: b.bottom + 4,
      items: [
        { label: t("db.copyCsv"), icon: "copy", action: () => void copyCsv() },
        "sep",
        { label: t("db.saveCsv"), icon: "download", action: () => void exportAs("csv") },
        { label: t("db.saveJson"), icon: "download", action: () => void exportAs("json") },
        { label: t("db.saveSql"), icon: "download", action: () => void exportAs("sql") },
      ],
    };
  }

  $effect(() => {
    if (focused && phase === "live") editor?.focus();
  });

  $effect(() => {
    if (phase !== "live" || !editorHost || editor) return;
    editor = createSqlEditor(editorHost, {
      doc: qtabs[qcur]?.sql ?? "",
      dialect: info?.kind ?? "mysql",
      onRun: () => void run(),
      onRunCurrent: () => void runCurrent(),
      onChange: (text) => {
        if (!qtabs[qcur]) return;
        qtabs[qcur].sql = text;
        persistTabs();
      },
    });
    updateCompletion();
  });

  onMount(() => {
    // The backend knows whether a password is saved (an identity's counts): it answers
    // `password_required` when one has to be asked.
    void connect();
    return () => {
      closed = true;
      editor?.destroy();
      editor = null;
      if (id) void api.dbClose(id).catch(() => {});
      id = null;
    };
  });

  const current = $derived(output?.results[active] ?? null);
</script>

<div class="db">
  {#if phase === "live"}
    <aside class="schema" aria-label={t("db.schema")}>
      <div class="schema-head">
        <Icon name="database" size={13} />
        <span class="title" title={info?.version}>{info?.database ?? conn?.name ?? ""}</span>
        <button class="icon-btn" title={t("db.refresh")} aria-label={t("db.refresh")} onclick={() => ((tables = {}), (columns = {}), void loadNamespaces())}
          ><Icon name="refresh" size={12} /></button
        >
      </div>
      <ul class="tree">
        {#each namespaces as ns (ns)}
          <li>
            <div class="nsrow">
              <button class="node ns" onclick={() => toggleNs(ns)} aria-expanded={!!openNs[ns]}>
                <Icon name={openNs[ns] ? "chevronDown" : "chevronRight"} size={11} /><Icon name="folder" size={12} />{ns}
              </button>
              <button class="icon-btn over" title={t("db.overview")} aria-label={t("db.overviewOf", { ns })} onclick={() => overview(ns)}><Icon name="info" size={12} /></button>
            </div>
            {#if openNs[ns]}
              {@const list = tables[ns]}
              <ul>
                {#if list === "loading" || list === undefined}
                  <li class="hint">{t("db.loading")}</li>
                {:else if !list.length}
                  <li class="hint">{t("db.noTables")}</li>
                {:else}
                  {#each list as tb (tb.name)}
                    {@const key = `${ns}.${tb.name}`}
                    <li>
                      <div class="node tb">
                        <button class="twisty" aria-label={t("db.columns")} aria-expanded={!!openTables[key]} onclick={() => toggleTable(ns, tb.name)}
                          ><Icon name={openTables[key] ? "chevronDown" : "chevronRight"} size={11} /></button
                        >
                        <button
                          class="tbname"
                          title={[t("db.openTable"), tableStats(tb), tb.comment].filter(Boolean).join("\n")}
                          onclick={() => openTable(ns, tb.name)}
                          ondblclick={() => openTable(ns, tb.name)}
                        >
                          <Icon name={tb.view ? "eye" : "layout"} size={12} />{tb.name}
                        </button>
                      </div>
                      {#if openTables[key]}
                        {@const cols = columns[key]}
                        <ul>
                          {#if !Array.isArray(cols)}
                            <li class="hint">{t("db.loading")}</li>
                          {:else}
                            {#each cols as c (c.name)}
                              <li class="col" title={`${c.data_type}${c.nullable ? "" : " NOT NULL"}${c.default ? ` DEFAULT ${c.default}` : ""}`}>
                                {#if c.primary_key}<Icon name="key" size={11} />{:else}<span class="dot"></span>{/if}<span class="cname">{c.name}</span><span class="ctype">{c.data_type}</span>
                              </li>
                            {/each}
                          {/if}
                        </ul>
                      {/if}
                    </li>
                  {/each}
                {/if}
              </ul>
            {/if}
          </li>
        {/each}
      </ul>
    </aside>
    <section class="main">
      <div class="toolbar">
        <button class="btn primary sm" onclick={run} disabled={running} title={t("db.runHint")}><Icon name="play" size={12} />{t("db.run")}</button>
        <button class="btn sm" onclick={runCurrent} disabled={running} title={t("db.runCurrentHint")}>{t("db.runCurrent")}</button>
        <button class="btn sm ghost" onclick={openHistory} aria-haspopup="menu" title={t("db.history")}><Icon name="clock" size={12} />{t("db.history")}</button>
        {#if running}<button class="btn sm" onclick={stop}><Icon name="stop" size={12} />{t("db.stop")}</button>{/if}
        <label class="rows">
          {t("db.maxRows")}
          <select bind:value={maxRows}>
            {#each ROW_CHOICES as n (n)}<option value={n}>{n.toLocaleString()}</option>{/each}
          </select>
        </label>
        <span class="spacer"></span>
        <button class="btn sm share" class:on={shared} onclick={toggleShare} title={t("db.shareHint")} aria-pressed={shared}
          ><Icon name="sparkle" size={12} />{shared ? t("db.shared") : t("db.share")}</button
        >
        <span class="ver" title={info?.version}>{info?.kind === "postgres" ? "PostgreSQL" : "MySQL"} · {conn?.user}@{conn?.host}</span>
      </div>
      <div class="qtabs" role="tablist" aria-label={t("db.queryTabs")}>
        {#each qtabs as q, i (q.key)}
          <div class="qtab" class:on={i === qcur}>
            <button role="tab" aria-selected={i === qcur} onclick={() => switchTab(i)} disabled={running && i !== qcur}>{q.name}</button>
            {#if qtabs.length > 1}<button class="x" aria-label={t("db.closeQuery", { name: q.name })} onclick={() => closeTab(i)} disabled={running}><Icon name="x" size={10} /></button>{/if}
          </div>
        {/each}
        <button class="icon-btn" aria-label={t("db.newQuery")} title={t("db.newQuery")} onclick={newTab} disabled={running}><Icon name="plus" size={12} /></button>
      </div>
      <div class="editor" bind:this={editorHost}></div>
      <div class="results">
        {#if browse}
          {@const info = tableInfo(browse.ns, browse.table)}
          {@const shown = output?.results[0]?.rows.length ?? 0}
          <form class="browse" onsubmit={(e) => (e.preventDefault(), applyFilter())}>
            <span class="where mono">WHERE</span>
            <input
              class="input mono"
              bind:value={filterText}
              placeholder={t("db.filterHint")}
              aria-label={t("db.filter")}
              spellcheck="false"
              autocapitalize="off"
            />
            {#if browse.filter}<button type="button" class="icon-btn" title={t("db.filterClear")} aria-label={t("db.filterClear")} onclick={() => ((filterText = ""), applyFilter())}><Icon name="x" size={12} /></button>{/if}
            <button type="submit" class="btn sm" disabled={running}>{t("db.filterApply")}</button>
            <span class="pager">
              <button type="button" class="icon-btn" disabled={running || browse.offset === 0} title={t("db.pagePrev")} aria-label={t("db.pagePrev")} onclick={() => page(-1)}>‹</button>
              <span class="mono">{shown ? `${(browse.offset + 1).toLocaleString()}–${(browse.offset + shown).toLocaleString()}` : "0"}</span>
              <button type="button" class="icon-btn" disabled={running || !hasMore} title={t("db.pageNext")} aria-label={t("db.pageNext")} onclick={() => page(1)}>›</button>
            </span>
            {#if info && tableStats(info)}<span class="stats" title={info.comment ?? undefined}>{tableStats(info)}</span>{/if}
            <button type="button" class="btn sm ghost" onclick={() => showDdl(browse!.ns, browse!.table)}><Icon name="file" size={12} />{t("db.ddl")}</button>
          </form>
        {/if}
        {#if ddl}
          <div class="ddl">
            <div class="ddl-head">
              <strong class="mono">{ddl.title}</strong>
              <span class="spacer"></span>
              <button class="btn sm ghost" onclick={() => ddl && void copyText(ddl.text).then((ok) => ok && toasts.success(t("db.ddlCopied")))}><Icon name="copy" size={12} />{t("edit.copy")}</button>
              <button class="btn sm ghost" onclick={() => ddl && (editor?.append(ddl.text), (ddl = null))}><Icon name="edit" size={12} />{t("db.ddlToEditor")}</button>
              <button class="icon-btn" aria-label={t("common.close")} onclick={() => (ddl = null)}><Icon name="x" size={13} /></button>
            </div>
            <pre class="mono">{ddl.text}</pre>
          </div>
        {/if}
        {#each asks as a (a.approval_id)}
          <div class="ask risk-{a.risk}" role="alertdialog" aria-label={t("db.askTitle", { who: a.actor_label })}>
            <div class="ask-head">
              <Icon name="sparkle" size={13} />
              <strong>{t("db.askTitle", { who: a.actor_label })}</strong>
              <span class="pill">{t(`db.risk.${a.risk}`)}</span>
              <span class="note">{a.policy_note}{a.statements > 1 ? ` · ${t("db.statements", { n: a.statements })}` : ""}</span>
            </div>
            {#if a.reason}<p class="why">{a.reason}</p>{/if}
            <pre class="sql">{a.sql}</pre>
            <div class="ask-actions">
              <input class="input" placeholder={t("db.denyNote")} bind:value={denyNote[a.approval_id]} aria-label={t("db.denyNote")} />
              <button class="btn sm" onclick={() => decide(a.approval_id, false)}>{t("db.deny")}</button>
              <button class="btn sm primary" class:danger={a.risk === "destructive"} onclick={() => decide(a.approval_id, true)}
                >{t("db.approve")}</button
              >
            </div>
          </div>
        {/each}
        {#if running}
          <div class="status"><span class="spinner"></span>{t("db.running")}</div>
        {:else if output}
          <div class="status" class:err={!!output.error}>
            {#if output.results.length > 1}
              <div class="tabs" role="tablist">
                {#each output.results as r, i (i)}
                  {@const s = resultSummary(r)}
                  <button role="tab" class:on={i === active} aria-selected={i === active} onclick={() => (active = i)}>
                    {i + 1} · {s.isQuery ? t("db.rows", { n: s.rows }) : t("db.affected", { n: s.affected })}
                  </button>
                {/each}
              </div>
            {/if}
            {#if edits && current}
              <span class="edit-tools">
                <button class="btn sm ghost" onclick={addRow} disabled={saving} title={t("db.addRow")}><Icon name="plus" size={12} />{t("db.addRow")}</button>
                <button class="btn sm ghost" onclick={deleteRow} disabled={!selected || saving} title={t("db.deleteRow")}><Icon name="trash" size={12} />{t("db.deleteRow")}</button>
                <button class="btn sm ghost" onclick={setNull} disabled={!selected || saving} title={t("db.setNull")}>NULL</button>
                {#if pending}
                  <button class="btn sm primary" onclick={save} disabled={saving}><Icon name="check" size={12} />{t("db.save", { n: pending })}</button>
                  <button class="btn sm" onclick={() => ((edits = noEdits()), (selected = null))} disabled={saving}>{t("db.discard")}</button>
                {/if}
              </span>
            {/if}
            {#if current}
              {@const s = resultSummary(current)}
              <span class="sum">
                {#if s.isQuery}{t("db.rows", { n: s.rows })}{#if s.truncated}<span class="pill warn">{t("db.truncated", { n: s.rows })}</span>{/if}{:else}{t(
                    "db.affected",
                    { n: s.affected },
                  )}{/if}
                · {output.elapsed_ms} ms
              </span>
              {#if s.isQuery && s.rows}<button class="btn sm ghost" onclick={openExport} aria-haspopup="menu"><Icon name="download" size={12} />{t("db.export")}</button>{/if}
            {/if}
            <span class="ran mono" title={ranLabel}>{ranLabel}</span>
          </div>
          {#if output.error}<p class="error" role="alert">{output.error}</p>{/if}
          {#if current && current.columns.length}
            <div class="gridbox">
              {#if edits && active === 0}
                <DbGrid
                  result={current}
                  bind:edits
                  bind:selected
                  bind:this={grid}
                  locked={saving}
                  sort={browse && { column: browse.orderBy, desc: browse.desc }}
                  onsort={browse ? sortBy : undefined}
                  firstRow={browse?.offset ?? 0}
                />
              {:else if browse}
                <DbGrid result={current} sort={{ column: browse.orderBy, desc: browse.desc }} onsort={sortBy} firstRow={browse.offset} />
              {:else}
                <DbGrid result={current} />
              {/if}
            </div>
          {/if}
        {:else}
          <p class="hint center">{t("db.empty")}</p>
        {/if}
      </div>
    </section>
  {:else}
    <div class="state" role="status">
      <Icon name="database" size={28} />
      {#if phase === "connecting"}
        <p>{t("db.connecting", { name: conn?.name ?? "" })}</p>
        <p class="hint mono">{conn ? `${conn.user}@${conn.host}:${conn.port}` : ""}</p>
      {:else if phase === "password"}
        <form
          class="pw"
          onsubmit={(e) => {
            e.preventDefault();
            // An empty password is a valid answer (servers that take none).
            void connect(password);
          }}
        >
          <p>{t("db.passwordFor", { user: conn?.user ?? "", name: conn?.name ?? "" })}</p>
          <!-- svelte-ignore a11y_autofocus -->
          <input class="input" type="password" bind:value={password} autocomplete="off" autofocus aria-label={t("db.password")} />
          <button class="btn primary sm" type="submit">{t("db.connect")}</button>
        </form>
      {:else}
        <p>{t("db.failed", { name: conn?.name ?? "" })}</p>
        {#if problem}<p class="error">{problem}</p>{/if}
        <button class="btn primary sm" onclick={() => connect()}
          ><Icon name="refresh" size={13} />{t("db.retry")}</button
        >
      {/if}
    </div>
  {/if}
</div>

{#if menu}<ContextMenu {...menu} onclose={() => (menu = null)} />{/if}

<style>
  .db {
    display: grid;
    grid-template-columns: 240px minmax(0, 1fr);
    height: 100%;
    min-height: 0;
    overflow: hidden;
    background: var(--bg);
  }
  .schema {
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-right: 1px solid var(--border);
    background: var(--panel);
  }
  .schema-head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
    font-weight: 600;
  }
  .schema-head .title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tree,
  .tree ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .tree {
    overflow: auto;
    padding: 4px 0;
    font-size: 12.5px;
  }
  .tree ul {
    padding-left: 14px;
  }
  .node {
    display: flex;
    align-items: center;
    gap: 4px;
    width: 100%;
    min-height: 24px;
    padding: 0 8px;
    border: 0;
    background: none;
    color: var(--text);
    text-align: left;
    cursor: pointer;
    font: inherit;
  }
  button.node:hover,
  .tbname:hover {
    background: var(--hover);
  }
  .node.tb {
    padding: 0;
  }
  .twisty,
  .tbname {
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
    cursor: pointer;
    min-height: 24px;
    display: flex;
    align-items: center;
    gap: 5px;
  }
  .twisty {
    padding: 0 2px 0 8px;
  }
  .tbname {
    flex: 1;
    min-width: 0;
    padding: 0 6px 0 2px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .col {
    display: flex;
    align-items: center;
    gap: 5px;
    min-height: 22px;
    padding: 0 8px 0 22px;
    color: var(--muted);
    font-family: var(--font-mono);
    font-size: 11.5px;
  }
  .col .cname {
    color: var(--text);
  }
  .col .ctype {
    margin-left: auto;
    color: var(--faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dot {
    width: 11px;
    flex: none;
  }
  .hint {
    color: var(--muted);
    padding: 3px 10px;
    font-size: 12px;
  }
  .main {
    display: grid;
    grid-template-rows: auto auto minmax(90px, 34%) minmax(0, 1fr);
    grid-template-columns: minmax(0, 1fr);
    min-width: 0;
    min-height: 0;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
  }
  .rows {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--muted);
    font-size: 12px;
  }
  .spacer {
    flex: 1;
  }
  .ver {
    color: var(--faint);
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .editor {
    min-height: 0;
    overflow: hidden;
    border-bottom: 1px solid var(--border);
  }
  .results {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .status {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 5px 8px;
    border-bottom: 1px solid var(--border);
    font-size: 12px;
    color: var(--muted);
    flex-wrap: wrap;
  }
  .tabs {
    display: flex;
    gap: 4px;
  }
  .tabs button {
    border: 1px solid var(--border);
    background: none;
    color: var(--muted);
    border-radius: 4px;
    padding: 1px 6px;
    font: inherit;
    cursor: pointer;
  }
  .tabs button.on {
    color: var(--text);
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  .qtabs {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 0 6px;
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
    font-size: 12px;
  }
  .qtab {
    display: flex;
    align-items: center;
    border-bottom: 2px solid transparent;
  }
  .qtab.on {
    border-bottom-color: var(--accent);
  }
  .qtab button {
    border: 0;
    background: none;
    color: var(--muted);
    font: inherit;
    padding: 5px 6px;
    cursor: pointer;
    white-space: nowrap;
  }
  .qtab.on button {
    color: var(--text);
  }
  .qtab .x {
    padding: 2px;
    opacity: 0.6;
  }
  .qtab .x:hover {
    opacity: 1;
  }
  .nsrow {
    display: flex;
    align-items: center;
  }
  .nsrow .node {
    flex: 1;
    min-width: 0;
  }
  .nsrow .over {
    opacity: 0;
    margin-right: 4px;
  }
  .nsrow:hover .over,
  .nsrow .over:focus-visible {
    opacity: 1;
  }
  .browse {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 8px;
    border-bottom: 1px solid var(--border);
    flex-wrap: wrap;
  }
  .browse .where {
    color: var(--faint);
    font-size: 11px;
  }
  .browse .input {
    flex: 1;
    min-width: 160px;
    height: 26px;
    font-size: 12px;
  }
  .pager {
    display: flex;
    align-items: center;
    gap: 2px;
    font-size: 12px;
    color: var(--muted);
  }
  .stats {
    color: var(--faint);
    font-size: 12px;
    white-space: nowrap;
  }
  .ddl {
    display: flex;
    flex-direction: column;
    min-height: 0;
    max-height: 60%;
    margin: 8px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--panel);
  }
  .ddl-head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
  }
  .ddl pre {
    margin: 0;
    padding: 10px;
    overflow: auto;
    font-size: 12px;
    white-space: pre;
  }
  .share.on {
    color: var(--accent);
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  .ask {
    display: grid;
    gap: 6px;
    margin: 8px;
    padding: 10px;
    border: 1px solid var(--warn);
    border-radius: 8px;
    background: var(--warn-soft);
  }
  .ask.risk-destructive {
    border-color: var(--danger);
    background: var(--danger-soft);
  }
  .ask.risk-read {
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  .ask-head {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    font-size: 12.5px;
  }
  .ask .pill {
    padding: 0 6px;
    border-radius: 4px;
    border: 1px solid currentColor;
    font-size: 11px;
    text-transform: uppercase;
  }
  .ask .note,
  .ask .why {
    color: var(--muted);
    font-size: 12px;
    margin: 0;
  }
  .ask .sql {
    margin: 0;
    max-height: 160px;
    overflow: auto;
    padding: 8px;
    border-radius: 6px;
    background: var(--bg);
    font-family: var(--font-mono);
    font-size: 12px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .ask-actions {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .ask-actions .input {
    flex: 1;
    min-width: 0;
  }
  .btn.danger {
    background: var(--danger);
    border-color: var(--danger);
  }
  .edit-tools {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
  }
  .sum {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text);
  }
  .ran {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--faint);
    text-align: right;
  }
  .error {
    margin: 0;
    padding: 8px 10px;
    color: var(--danger);
    background: var(--danger-soft);
    font-family: var(--font-mono);
    font-size: 12px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .gridbox {
    flex: 1;
    min-height: 0;
  }
  .center {
    margin: auto;
    text-align: center;
  }
  .spinner {
    width: 12px;
    height: 12px;
    border: 2px solid var(--border-strong);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .state {
    grid-column: 1 / -1;
    display: grid;
    place-content: center;
    justify-items: center;
    gap: 8px;
    padding: 24px;
    text-align: center;
    color: var(--muted);
  }
  .state p {
    margin: 0;
  }
  .state .error {
    max-width: 520px;
    border-radius: 6px;
  }
  .pw {
    display: grid;
    gap: 8px;
    justify-items: center;
  }
  .pw .input {
    width: 260px;
  }
</style>
