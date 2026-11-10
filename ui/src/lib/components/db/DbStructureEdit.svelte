<script lang="ts">
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";
  import { FK_RULES, isGenerated, type TableDef } from "../../util/ddl";

  /** Edit a table's columns, indexes and foreign keys (or a new table's). Nothing runs here:
   *  the parent turns the edited definition into SQL and shows it first. */
  let {
    def = $bindable(),
    name = $bindable(""),
    creating = false,
    dialect,
    namespace,
    tables,
    problem,
    statements,
    onreview,
    oncancel,
  }: {
    def: TableDef;
    /** The new table's name (creating only). */
    name?: string;
    creating?: boolean;
    dialect: "mysql" | "postgres";
    namespace: string;
    /** Tables of the namespace, for foreign keys. */
    tables: string[];
    /** Why the definition cannot be saved yet (an i18n key suffix), or null. */
    problem: string | null;
    /** How many statements saving would run. */
    statements: number;
    onreview: () => void;
    oncancel: () => void;
  } = $props();

  const list = (s: string) =>
    s
      .split(",")
      .map((x) => x.trim())
      .filter(Boolean);
  const columnNames = $derived(def.columns.map((c) => c.name.trim()).filter(Boolean));

  function addColumn() {
    def.columns.push({ orig: null, name: "", type: dialect === "mysql" ? "varchar(255)" : "text", nullable: true, default: null, comment: null, extra: null });
  }
  function addIndex() {
    def.indexes.push({ orig: null, name: "", columns: [], unique: false, primary: false });
  }
  function addFk() {
    def.fks.push({ orig: null, name: "", columns: [], refNs: namespace, refTable: "", refColumns: [], onUpdate: "NO ACTION", onDelete: "NO ACTION" });
  }
</script>

<div class="edit">
  {#if creating}
    <label class="tname">
      <span>{t("db.st.tableName")}</span>
      <!-- svelte-ignore a11y_autofocus -->
      <input class="input mono" bind:value={name} autofocus spellcheck="false" autocapitalize="off" />
    </label>
  {/if}

  <section>
    <h3>{t("db.st.columns")}</h3>
    <table>
      <thead>
        <tr><th>{t("db.st.name")}</th><th>{t("db.st.type")}</th><th>{t("db.st.null")}</th><th>{t("db.st.default")}</th><th>{t("db.st.comment")}</th><th>{t("db.st.extra")}</th><th></th></tr>
      </thead>
      <tbody>
        {#each def.columns as c, i (i)}
          {@const locked = isGenerated(c)}
          <tr class:new={c.orig === null}>
            <td><input class="input mono" bind:value={c.name} aria-label={t("db.st.name")} spellcheck="false" disabled={locked} /></td>
            <td><input class="input mono" bind:value={c.type} aria-label={t("db.st.type")} spellcheck="false" disabled={locked} /></td>
            <td class="c"><input type="checkbox" bind:checked={c.nullable} aria-label={t("db.st.null")} disabled={locked} /></td>
            <td>
              <input
                class="input mono"
                value={c.default ?? ""}
                oninput={(e) => (c.default = e.currentTarget.value === "" ? null : e.currentTarget.value)}
                placeholder={t("db.st.noDefault")}
                aria-label={t("db.st.default")}
                spellcheck="false"
                disabled={locked}
              />
            </td>
            <td>
              <input
                class="input"
                value={c.comment ?? ""}
                oninput={(e) => (c.comment = e.currentTarget.value === "" ? null : e.currentTarget.value)}
                aria-label={t("db.st.comment")}
                disabled={locked}
              />
            </td>
            <td class="mono faint small" title={locked ? t("db.st.generatedLocked") : undefined}>{c.extra ?? ""}</td>
            <td><button class="icon-btn" aria-label={t("db.st.remove")} title={t("db.st.remove")} onclick={() => def.columns.splice(i, 1)}><Icon name="trash" size={12} /></button></td>
          </tr>
        {/each}
      </tbody>
    </table>
    <button class="btn sm ghost" onclick={addColumn}><Icon name="plus" size={12} />{t("db.st.addColumn")}</button>
  </section>

  <section>
    <h3>{t("db.st.indexes")}</h3>
    <table>
      <thead><tr><th>{t("db.st.name")}</th><th>{t("db.st.columnsList")}</th><th>{t("db.st.unique")}</th><th>{t("db.st.primary")}</th><th></th></tr></thead>
      <tbody>
        {#each def.indexes as x, i (i)}
          <tr class:new={x.orig === null}>
            {#if x.orig === null}
              <td><input class="input mono" bind:value={x.name} disabled={x.primary} placeholder={x.primary ? "PRIMARY" : "ix_name"} aria-label={t("db.st.name")} /></td>
              <td>
                <input
                  class="input mono"
                  value={x.columns.join(", ")}
                  oninput={(e) => (x.columns = list(e.currentTarget.value))}
                  placeholder={columnNames.slice(0, 2).join(", ")}
                  aria-label={t("db.st.columnsList")}
                  list="db-st-cols"
                />
              </td>
              <td class="c"><input type="checkbox" bind:checked={x.unique} disabled={x.primary} aria-label={t("db.st.unique")} /></td>
              <td class="c"><input type="checkbox" bind:checked={x.primary} onchange={() => x.primary && (x.unique = true)} aria-label={t("db.st.primary")} /></td>
            {:else}
              <td class="mono">{x.name}</td>
              <td class="mono">{x.columns.join(", ")}</td>
              <td class="c">{x.unique ? "✓" : ""}</td>
              <td class="c">{x.primary ? "✓" : ""}</td>
            {/if}
            <td><button class="icon-btn" aria-label={t("db.st.remove")} title={t("db.st.remove")} onclick={() => def.indexes.splice(i, 1)}><Icon name="trash" size={12} /></button></td>
          </tr>
        {/each}
      </tbody>
    </table>
    <button class="btn sm ghost" onclick={addIndex}><Icon name="plus" size={12} />{t("db.st.addIndex")}</button>
  </section>

  <section>
    <h3>{t("db.st.foreignKeys")}</h3>
    <table>
      <thead><tr><th>{t("db.st.name")}</th><th>{t("db.st.columnsList")}</th><th>{t("db.st.refTable")}</th><th>{t("db.st.refColumns")}</th><th>{t("db.st.onUpdate")}</th><th>{t("db.st.onDelete")}</th><th></th></tr></thead>
      <tbody>
        {#each def.fks as f, i (i)}
          <tr class:new={f.orig === null}>
            {#if f.orig === null}
              <td><input class="input mono" bind:value={f.name} placeholder="fk_name" aria-label={t("db.st.name")} /></td>
              <td><input class="input mono" value={f.columns.join(", ")} oninput={(e) => (f.columns = list(e.currentTarget.value))} aria-label={t("db.st.columnsList")} list="db-st-cols" /></td>
              <td>
                <select class="select" bind:value={f.refTable} aria-label={t("db.st.refTable")}>
                  <option value="">—</option>
                  {#each tables as tb (tb)}<option value={tb}>{tb}</option>{/each}
                </select>
              </td>
              <td><input class="input mono" value={f.refColumns.join(", ")} oninput={(e) => (f.refColumns = list(e.currentTarget.value))} aria-label={t("db.st.refColumns")} /></td>
              <td><select class="select" bind:value={f.onUpdate} aria-label={t("db.st.onUpdate")}>{#each FK_RULES as r (r)}<option>{r}</option>{/each}</select></td>
              <td><select class="select" bind:value={f.onDelete} aria-label={t("db.st.onDelete")}>{#each FK_RULES as r (r)}<option>{r}</option>{/each}</select></td>
            {:else}
              <td class="mono">{f.name}</td>
              <td class="mono">{f.columns.join(", ")}</td>
              <td class="mono">{f.refNs}.{f.refTable}</td>
              <td class="mono">{f.refColumns.join(", ")}</td>
              <td>{f.onUpdate}</td>
              <td>{f.onDelete}</td>
            {/if}
            <td><button class="icon-btn" aria-label={t("db.st.remove")} title={t("db.st.remove")} onclick={() => def.fks.splice(i, 1)}><Icon name="trash" size={12} /></button></td>
          </tr>
        {/each}
      </tbody>
    </table>
    <button class="btn sm ghost" onclick={addFk}><Icon name="plus" size={12} />{t("db.st.addFk")}</button>
  </section>

  <datalist id="db-st-cols">{#each columnNames as n (n)}<option value={n}></option>{/each}</datalist>

  <footer>
    {#if problem}
      <span class="problem">{t(`db.st.problem.${problem}` as Parameters<typeof t>[0])}</span>
    {:else}
      <span class="faint">{t("db.st.statements", { n: statements })}</span>
    {/if}
    <span class="spacer"></span>
    <button class="btn sm ghost" onclick={oncancel}>{t("common.cancel")}</button>
    <button class="btn sm primary" onclick={onreview} disabled={!!problem || statements === 0}>{t("db.st.review")}</button>
  </footer>
</div>

<style>
  .edit {
    overflow: auto;
    padding: 8px 10px 0;
    min-height: 0;
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .tname {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    color: var(--muted);
  }
  .tname .input {
    max-width: 320px;
  }
  h3 {
    margin: 0 0 6px;
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--muted);
  }
  table {
    border-collapse: collapse;
    width: 100%;
    font-size: 12px;
    margin-bottom: 6px;
  }
  th {
    text-align: left;
    font-weight: 500;
    color: var(--faint);
    padding: 3px 4px;
    white-space: nowrap;
  }
  td {
    padding: 2px 4px;
    vertical-align: middle;
  }
  td .input,
  td .select {
    width: 100%;
    height: 26px;
    font-size: 12px;
  }
  tr.new td:first-child {
    box-shadow: inset 2px 0 0 var(--accent);
  }
  .c {
    text-align: center;
    width: 40px;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .faint {
    color: var(--faint);
  }
  .small {
    font-size: 11px;
    max-width: 160px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  footer {
    position: sticky;
    bottom: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 0;
    background: var(--bg);
    border-top: 1px solid var(--border);
    font-size: 12px;
  }
  .problem {
    color: var(--warn);
  }
  .spacer {
    flex: 1;
  }
</style>
