<script lang="ts">
  import type { DbTableStructure } from "../../api/types";
  import { t } from "../../i18n";
  import Icon from "../icons/Icon.svelte";

  /** A table's columns, indexes and foreign keys, read-only. */
  let { structure }: { structure: DbTableStructure } = $props();
</script>

<div class="structure">
  <section>
    <h3>{t("db.st.columns")} <span class="n">{structure.columns.length}</span></h3>
    <table>
      <thead>
        <tr><th></th><th>{t("db.st.name")}</th><th>{t("db.st.type")}</th><th>{t("db.st.null")}</th><th>{t("db.st.default")}</th><th>{t("db.st.extra")}</th><th>{t("db.st.comment")}</th></tr>
      </thead>
      <tbody>
        {#each structure.columns as c (c.name)}
          <tr>
            <td class="key">{#if c.primary_key}<Icon name="key" size={11} />{/if}</td>
            <td class="mono strong">{c.name}</td>
            <td class="mono">{c.data_type}</td>
            <td>{c.nullable ? t("db.st.yes") : ""}</td>
            <td class="mono" class:faint={c.default === null}>{c.default ?? (c.nullable ? "NULL" : "")}</td>
            <td class="mono faint">{c.extra ?? ""}</td>
            <td>{c.comment ?? ""}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </section>

  <section>
    <h3>{t("db.st.indexes")} <span class="n">{structure.indexes.length}</span></h3>
    {#if structure.indexes.length}
      <table>
        <thead><tr><th>{t("db.st.name")}</th><th>{t("db.st.columns")}</th><th>{t("db.st.kind")}</th></tr></thead>
        <tbody>
          {#each structure.indexes as i (i.name)}
            <tr>
              <td class="mono strong">{i.name}</td>
              <td class="mono">{i.columns.join(", ")}</td>
              <td>
                {#if i.primary}<span class="pill">{t("db.st.primary")}</span>{:else if i.unique}<span class="pill">{t("db.st.unique")}</span>{/if}
                <span class="faint">{i.kind.toLowerCase()}</span>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {:else}
      <p class="none">{t("db.st.none")}</p>
    {/if}
  </section>

  <section>
    <h3>{t("db.st.foreignKeys")} <span class="n">{structure.foreign_keys.length}</span></h3>
    {#if structure.foreign_keys.length}
      <table>
        <thead><tr><th>{t("db.st.name")}</th><th>{t("db.st.columns")}</th><th>{t("db.st.references")}</th><th>{t("db.st.onUpdate")}</th><th>{t("db.st.onDelete")}</th></tr></thead>
        <tbody>
          {#each structure.foreign_keys as f (f.name)}
            <tr>
              <td class="mono strong">{f.name}</td>
              <td class="mono">{f.columns.join(", ")}</td>
              <td class="mono">{f.ref_namespace}.{f.ref_table} ({f.ref_columns.join(", ")})</td>
              <td>{f.on_update}</td>
              <td>{f.on_delete}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {:else}
      <p class="none">{t("db.st.none")}</p>
    {/if}
  </section>
</div>

<style>
  .structure {
    overflow: auto;
    padding: 8px 10px 16px;
    min-height: 0;
    flex: 1;
  }
  section + section {
    margin-top: 14px;
  }
  h3 {
    margin: 0 0 6px;
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--muted);
  }
  .n {
    color: var(--faint);
    font-weight: 400;
  }
  table {
    border-collapse: collapse;
    width: 100%;
    font-size: 12px;
  }
  th {
    text-align: left;
    font-weight: 500;
    color: var(--faint);
    border-bottom: 1px solid var(--border);
    padding: 4px 8px;
    white-space: nowrap;
  }
  td {
    padding: 4px 8px;
    border-bottom: 1px solid var(--border);
    vertical-align: top;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .strong {
    color: var(--text);
  }
  .faint {
    color: var(--faint);
  }
  .key {
    width: 14px;
    color: var(--warn);
  }
  .pill {
    padding: 0 5px;
    margin-right: 4px;
    border: 1px solid var(--border-strong);
    border-radius: 4px;
    font-size: 11px;
  }
  .none {
    margin: 0;
    color: var(--faint);
    font-size: 12px;
  }
</style>
