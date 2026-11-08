<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "../icons/Icon.svelte";
  import DbGrid from "./DbGrid.svelte";
  import { api } from "../../api";
  import type { DbColumnInfo, DbOpened, DbQueryOutput, DbTableInfo } from "../../api/types";
  import { errorMessage, isIpcError } from "../../api/errors";
  import { t } from "../../i18n";
  import { connections } from "../../stores/connections.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { copyText } from "../../util/clipboard";
  import { createSqlEditor, type SqlEditorHandle } from "../../util/codemirror";
  import { primaryResult, resultSummary, toCsv } from "../../util/db";

  /** A database in a pane: schema on the left, SQL editor over the results on the right. */
  let { connectionId, focused }: { connectionId: string; focused: boolean } = $props();

  const conn = $derived(connections.byId.get(connectionId));
  const sqlKey = $derived(`handrail.db.sql.${connectionId}`);
  const ROW_CHOICES = [100, 1000, 10000];

  let phase = $state<"connecting" | "password" | "live" | "failed">("connecting");
  let problem = $state<string | null>(null);
  let password = $state("");
  let info = $state<DbOpened | null>(null);
  let id: string | null = null;
  let closed = false;

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

  const storedSql = () => {
    try {
      return localStorage.getItem(sqlKey) ?? "";
    } catch {
      return "";
    }
  };

  async function connect(pw?: string) {
    phase = "connecting";
    problem = null;
    try {
      const opened = await api.dbOpen(connectionId, pw);
      if (closed) return void api.dbClose(opened.id).catch(() => {});
      id = opened.id;
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

  async function run() {
    if (!id || running || !editor) return;
    const sql = editor.runText().trim();
    if (!sql) return;
    ranLabel = sql.length > 80 ? `${sql.slice(0, 80)}…` : sql;
    await execute(() => api.dbQuery(id!, sql, maxRows));
  }

  async function openTable(ns: string, table: string) {
    if (!id || running) return;
    ranLabel = `${ns}.${table}`;
    await execute(() => api.dbTableRows(id!, ns, table, maxRows));
  }

  async function execute(call: () => Promise<DbQueryOutput>) {
    running = true;
    try {
      const out = await call();
      if (closed) return;
      output = out;
      active = Math.max(0, primaryResult(out));
    } catch (e) {
      output = { results: [], error: errorMessage(e), elapsed_ms: 0 };
    } finally {
      running = false;
    }
  }

  async function stop() {
    if (id) await api.dbCancel(id).catch((e) => toasts.error(e));
  }

  async function copyCsv() {
    const r = output?.results[active];
    if (!r) return;
    if (await copyText(toCsv(r))) toasts.success(t("db.csvCopied", { rows: r.rows.length }));
  }

  $effect(() => {
    if (focused && phase === "live") editor?.focus();
  });

  $effect(() => {
    if (phase !== "live" || !editorHost || editor) return;
    editor = createSqlEditor(editorHost, {
      doc: storedSql(),
      dialect: info?.kind ?? "mysql",
      onRun: () => void run(),
      onChange: (text) => {
        try {
          localStorage.setItem(sqlKey, text);
        } catch {
          // Not saved between sessions; nothing else depends on it.
        }
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
            <button class="node ns" onclick={() => toggleNs(ns)} aria-expanded={!!openNs[ns]}>
              <Icon name={openNs[ns] ? "chevronDown" : "chevronRight"} size={11} /><Icon name="folder" size={12} />{ns}
            </button>
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
                        <button class="tbname" title={t("db.openTable")} onclick={() => openTable(ns, tb.name)} ondblclick={() => openTable(ns, tb.name)}>
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
        {#if running}<button class="btn sm" onclick={stop}><Icon name="stop" size={12} />{t("db.stop")}</button>{/if}
        <label class="rows">
          {t("db.maxRows")}
          <select bind:value={maxRows}>
            {#each ROW_CHOICES as n (n)}<option value={n}>{n.toLocaleString()}</option>{/each}
          </select>
        </label>
        <span class="spacer"></span>
        <span class="ver" title={info?.version}>{info?.kind === "postgres" ? "PostgreSQL" : "MySQL"} · {conn?.user}@{conn?.host}</span>
      </div>
      <div class="editor" bind:this={editorHost}></div>
      <div class="results">
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
            {#if current}
              {@const s = resultSummary(current)}
              <span class="sum">
                {#if s.isQuery}{t("db.rows", { n: s.rows })}{#if s.truncated}<span class="pill warn">{t("db.truncated", { n: s.rows })}</span>{/if}{:else}{t(
                    "db.affected",
                    { n: s.affected },
                  )}{/if}
                · {output.elapsed_ms} ms
              </span>
              {#if s.isQuery && s.rows}<button class="btn sm ghost" onclick={copyCsv}><Icon name="copy" size={12} />{t("db.copyCsv")}</button>{/if}
            {/if}
            <span class="ran mono" title={ranLabel}>{ranLabel}</span>
          </div>
          {#if output.error}<p class="error" role="alert">{output.error}</p>{/if}
          {#if current && current.columns.length}
            <div class="gridbox"><DbGrid result={current} /></div>
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
    grid-template-rows: auto minmax(90px, 34%) minmax(0, 1fr);
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
